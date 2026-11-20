//! The egress rules FlowR gives `flowr_policy::egress` (RT-1): a project's org's
//! enforced egress files (`egress/*.yaml`) and the agent's own list
//! (`egress:` in its own policy). Read at most every 30 s per project and agent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flowr_policy::egress::{EgressRules, Rules};
use flowr_storage::StorageBackend;

use crate::handlers::call_lancedb::{query_platform_table, sql_quote};

const FRESH: Duration = Duration::from_secs(30);

pub struct OrgEgress {
    storage: Arc<dyn StorageBackend>,
    cache: Mutex<HashMap<(String, String), (Instant, Rules)>>,
}

pub fn install(storage: Arc<dyn StorageBackend>) {
    flowr_policy::egress::set_rules(Arc::new(OrgEgress { storage, cache: Default::default() }));
}

fn col(row: &serde_json::Value, k: &str) -> String {
    row.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

#[async_trait::async_trait]
impl EgressRules for OrgEgress {
    async fn rules(&self, project: &str, agent: Option<&str>) -> Rules {
        let key = (project.to_string(), agent.unwrap_or_default().to_string());
        if let Some((at, r)) = self.cache.lock().unwrap().get(&key) {
            if at.elapsed() < FRESH {
                return r.clone();
            }
        }
        let s = self.storage.as_ref();
        let mut rules = Rules::default();
        let org = query_platform_table(s, "projects", Some(format!("namespace = {}", sql_quote(project))), 1).await.ok()
            .and_then(|r| r.into_iter().next())
            .filter(|p| col(p, "owner_kind") == "org")
            .map(|p| col(&p, "owner"));
        if let Some(org) = org {
            let mut hosts: Option<Vec<String>> = None;
            for (path, text) in crate::org_policy::enforced_files(s, &org).await {
                if flowr_policy::policy_file::kind_of_path(&path) != Some("egress") {
                    continue;
                }
                match flowr_policy::policy_file::EgressFile::parse(&text) {
                    Ok(f) => {
                        hosts.get_or_insert_with(Vec::new).extend(f.hosts);
                        rules.private.extend(f.private);
                    }
                    // Checked at save; one that no longer parses allows nothing.
                    Err(e) => {
                        tracing::warn!(org, path, error = %e, "egress file doesn't parse; its org reaches no host");
                        hosts.get_or_insert_with(Vec::new);
                    }
                }
            }
            rules.hosts = hosts;
        }
        if let Some(a) = agent.filter(|a| !a.is_empty()) {
            rules.agent_hosts = query_platform_table(s, crate::agent_space::security::TABLE, Some(format!("agent_id = {}", sql_quote(a))), 1).await.ok()
                .and_then(|r| r.into_iter().next())
                .and_then(|r| flowr_policy::policy_file::PolicyFile::parse(&col(&r, "policy")).ok())
                .and_then(|f| f.egress);
        }
        self.cache.lock().unwrap().insert(key, (Instant::now(), rules.clone()));
        rules
    }
}

/// The agent a run acts as, if any (`principal.template.agent`).
pub fn agent_of(ctx: &swf_runtime::context::ExecutionContext) -> Option<String> {
    ctx.template_meta.get("template").and_then(|t| t.get("agent")).and_then(|a| a.as_str()).map(str::to_string)
}

/// Refuse a call out of A3T that the egress rules don't allow, recording it.
pub async fn guard(principal: &flowr_policy::Principal, project: &str, agent: Option<&str>, task_type: &str, url: &str) -> Result<(), String> {
    match flowr_policy::egress::check(url, project, agent).await {
        Ok(()) => Ok(()),
        Err(why) => {
            flowr_policy::audit::event_decision(principal, "egress:call", "Url", url, Some(project), None,
                &flowr_policy::PolicyDecision::Deny { reason: why.clone(), policy_ids: vec!["egress".into()] },
                Some(&format!("{task_type}{}", agent.map(|a| format!(" by agent {a}")).unwrap_or_default())));
            tracing::warn!(%url, project, task_type, reason = %why, "outbound call refused (egress)");
            Err(format!("refused: {why}"))
        }
    }
}
