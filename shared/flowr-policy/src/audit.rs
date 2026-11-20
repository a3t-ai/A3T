//! Decision audit trail (E2 #1).
//!
//! Every policy decision — allow *and* deny — is recorded here as it is made at
//! the choke point. This is the CE-side, queryable audit log (a bounded
//! in-memory ring buffer exposed via `GET /security/audit`). The E0.5 `aesop.*`
//! `tool_call` spans remain the canonical tracing/OTel event source; EE's
//! richer exporter (aesop-tower `AuditLayer`, retention + PII redaction) is a
//! second subscriber layered on top — this buffer is the always-on baseline.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::{ExecutionRequest, PolicyDecision};

/// One authorization decision, flattened for the audit log.
#[derive(Debug, Clone, Serialize, serde::Deserialize, Default)]
pub struct AuditRecord {
    /// Unique per decision: `<pod>-<uuid v7>` (time ordered).
    #[serde(default)]
    pub id: String,
    /// Unix milliseconds when the decision was recorded.
    pub ts_ms: u64,
    /// The FlowR pod that decided (`HOSTNAME`).
    #[serde(default)]
    pub pod: String,
    pub principal_kind: String,
    pub principal_id: String,
    pub on_behalf_of: Option<String>,
    /// A SuperAdmin acting as `principal_id`: who really made the call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impersonated_by: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    /// `"allow"` or `"deny"`.
    pub decision: String,
    pub policy_ids: Vec<String>,
    pub reason: Option<String>,
    /// The project (namespace) the decision was about, when the call names one:
    /// a tool's `namespace` / `project` argument, a `ns/name` resource, or the
    /// resource's own namespace. `None` for platform-wide calls.
    pub project: Option<String>,
    /// The org whose log holds the decision (the project's org; `platform` for
    /// platform-wide calls and personal projects). Set by the durable log.
    #[serde(default)]
    pub org: Option<String>,
    /// The roles the principal carried into the decision (AUD-2).
    #[serde(default)]
    pub roles: Vec<String>,
    /// ID-2: the A3T agent acting (its id, not the session), the version it ran,
    /// its owner, and the flow it ran in. The person it acts for is `on_behalf_of`.
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_version: Option<String>,
    #[serde(default)]
    pub agent_owner: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub flow: Option<String>,
    /// The version (hash) of the policy set that decided (AUD-2).
    #[serde(default)]
    pub policy_version: Option<String>,
}

/// The resource attribute naming the org an event belongs to (see [`event`]).
const ORG_ATTR: &str = "a3t_audit_org";

/// Where each decision goes besides this pod's recent buffer: the durable,
/// cluster-wide log (flowr-ce `decision_log`). Set once at start.
static SINK: OnceLock<Box<dyn Fn(AuditRecord) + Send + Sync>> = OnceLock::new();

pub fn set_sink(sink: Box<dyn Fn(AuditRecord) + Send + Sync>) {
    let _ = SINK.set(sink);
}

fn pod() -> &'static str {
    static POD: OnceLock<String> = OnceLock::new();
    POD.get_or_init(|| std::env::var("HOSTNAME").ok().filter(|h| !h.is_empty()).unwrap_or_else(|| "local".into()))
}

/// (agent, session) from an agent principal's id `agent:<session>`, where an A3T
/// agent's session is `<agent-id>:<session>`.
pub fn agent_and_session(agent_id: &str) -> (Option<String>, Option<String>) {
    let session = agent_id.strip_prefix("agent:").unwrap_or(agent_id);
    match session.split_once(':') {
        Some((agent, _)) if !agent.is_empty() => (Some(agent.to_string()), Some(session.to_string())),
        _ => (None, Some(session.to_string()).filter(|s| !s.is_empty())),
    }
}

/// The project a request concerns, read from what the call itself names.
pub fn project_of(req: &ExecutionRequest) -> Option<String> {
    let s = |v: Option<&serde_json::Value>| v.and_then(|x| x.as_str()).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string);
    let p = &req.params;
    let from = |o: &serde_json::Value| s(o.get("namespace")).or_else(|| s(o.get("project")))
        .or_else(|| s(o.get("resource")).and_then(|r| r.split_once('/').map(|(ns, _)| ns.to_string())));
    from(p)
        .or_else(|| p.get("input").and_then(|i| from(i)))
        .or_else(|| p.get("arguments").and_then(|i| from(i)))
        .or_else(|| s(req.resource.attributes.get("namespace")))
        .or_else(|| req.resource.id.split_once('/').map(|(ns, _)| ns.to_string()).filter(|ns| !ns.is_empty() && !ns.contains(':')))
}

/// Ring-buffer capacity (records; oldest evicted).
const CAPACITY: usize = 1000;

static BUFFER: OnceLock<Mutex<VecDeque<AuditRecord>>> = OnceLock::new();

fn buffer() -> &'static Mutex<VecDeque<AuditRecord>> {
    BUFFER.get_or_init(|| Mutex::new(VecDeque::with_capacity(CAPACITY)))
}

/// Record a decision. Called at the choke point right after `check()`.
pub fn record(req: &ExecutionRequest, decision: &PolicyDecision) {
    record_noted(req, decision, None)
}

/// [`record`], with a note in `reason` when the decision carries none (for
/// example the reason a SuperAdmin gave for acting as someone).
pub fn record_noted(req: &ExecutionRequest, decision: &PolicyDecision, note: Option<&str>) {
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let (decision_s, policy_ids, reason) = match decision {
        PolicyDecision::Allow => ("allow".to_string(), Vec::new(), None),
        PolicyDecision::Deny { reason, policy_ids } => {
            ("deny".to_string(), policy_ids.clone(), Some(reason.clone()))
        }
        // An `Ask` is audited at the moment it is raised, separately from
        // whatever the human then answers — so the trail shows both that
        // approval was demanded and, later, what was granted.
        PolicyDecision::Ask { reason, policy_ids, .. } => {
            ("ask".to_string(), policy_ids.clone(), Some(reason.clone()))
        }
    };
    let (roles, agent, session, agent_owner, flow) = match &req.principal {
        crate::Principal::Human { roles, .. } => (roles.clone(), None, None, None, None),
        crate::Principal::Agent { agent_id, roles, owner, flow_id, .. } => {
            let (agent, session) = agent_and_session(&agent_id.0);
            (roles.clone(), agent, session, owner.clone(), flow_id.clone())
        }
        crate::Principal::Service { .. } => (Vec::new(), None, None, None, None),
    };
    let rec = AuditRecord {
        id: format!("{}-{}", pod(), uuid::Uuid::now_v7().simple()),
        pod: pod().to_string(),
        // An event about an org itself (its policy files) names the org.
        org: req.resource.attributes.get(ORG_ATTR).and_then(|v| v.as_str()).map(str::to_string),
        roles,
        agent,
        agent_version: None,
        agent_owner,
        session,
        flow,
        policy_version: crate::policy_hook().version_for(req),
        ts_ms,
        principal_kind: req.principal.kind().to_string(),
        principal_id: req.principal.id().to_string(),
        on_behalf_of: req.principal.on_behalf_of().map(String::from),
        impersonated_by: req.principal.impersonated_by().map(String::from),
        action: req.action.as_str().to_string(),
        resource_type: req.resource.type_name.clone(),
        resource_id: req.resource.id.clone(),
        decision: decision_s,
        policy_ids,
        reason,
        project: project_of(req),
    };
    let mut rec = rec;
    if rec.reason.is_none() {
        rec.reason = note.map(str::to_string);
    }
    if let Some(sink) = SINK.get() {
        sink(rec.clone());
    }
    if let Ok(mut b) = buffer().lock() {
        if b.len() >= CAPACITY {
            b.pop_front();
        }
        b.push_back(rec);
    }
}

/// The most recent `limit` decisions, newest first.
pub fn recent(limit: usize) -> Vec<AuditRecord> {
    match buffer().lock() {
        Ok(b) => b.iter().rev().take(limit).cloned().collect(),
        Err(_) => Vec::new(),
    }
}

/// Recent decisions filtered to denials only (the security-relevant subset).
pub fn recent_denials(limit: usize) -> Vec<AuditRecord> {
    match buffer().lock() {
        Ok(b) => b
            .iter()
            .rev()
            .filter(|r| r.decision == "deny")
            .take(limit)
            .cloned()
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{actions, AgentId, Principal, Resource};

    #[test]
    fn records_allow_and_deny() {
        let req = ExecutionRequest {
            principal: Principal::Agent {
                memory_branch: None,
                agent_id: AgentId("agt-1".into()),
                on_behalf_of: Some("user::sriram".into()),
                flow_id: None,
                roles: vec![],
                owner: None,
            },
            action: actions::tool_invoke(),
            resource: Resource::tool("call:rotate"),
            context: Default::default(),
            attributes: Default::default(),
            params: serde_json::json!({}),
        };
        record(&req, &PolicyDecision::Allow);
        record(
            &req,
            &PolicyDecision::Deny {
                reason: "destructive".into(),
                policy_ids: vec!["policy3".into()],
            },
        );
        let recent = recent(10);
        assert!(recent.len() >= 2);
        // newest first: the deny
        let deny = &recent[0];
        assert_eq!(deny.decision, "deny");
        assert_eq!(deny.principal_kind, "agent");
        assert_eq!(deny.on_behalf_of.as_deref(), Some("user::sriram"));
        assert_eq!(deny.resource_id, "call:rotate");
        assert_eq!(deny.policy_ids, vec!["policy3".to_string()]);
        assert!(recent_denials(10).iter().all(|r| r.decision == "deny"));
    }
}


#[cfg(test)]
mod project_tests {
    use super::project_of;
    use crate::{ExecutionRequest, Principal, Resource};
    fn req(params: serde_json::Value, rid: &str) -> ExecutionRequest {
        ExecutionRequest { principal: Principal::service("t"), action: crate::actions::tool_invoke(), resource: Resource::new("Tool", rid), context: Default::default(), attributes: Default::default(), params }
    }
    #[test]
    fn the_project_is_what_the_call_names() {
        assert_eq!(project_of(&req(serde_json::json!({"namespace": "acme"}), "list_flows")).as_deref(), Some("acme"));
        assert_eq!(project_of(&req(serde_json::json!({"input": {"project": "acme"}}), "x")).as_deref(), Some("acme"));
        assert_eq!(project_of(&req(serde_json::json!({"resource": "acme/claude"}), "call:llm")).as_deref(), Some("acme"));
        assert_eq!(project_of(&req(serde_json::json!({}), "acme/flow_x")).as_deref(), Some("acme"));
        assert_eq!(project_of(&req(serde_json::json!({}), "list_projects")), None);
        assert_eq!(project_of(&req(serde_json::json!({}), "call:llm")), None);
    }
}

/// Record something that happened outside a policy check (AUD-3): an approval
/// answered, a key made or revoked, a policy changed, a refusal by project
/// isolation, Act as stopped. `denied`: the reason, when it was refused.
pub fn event(
    principal: &crate::Principal,
    action: &str,
    resource_type: &str,
    resource_id: &str,
    project: Option<&str>,
    org: Option<&str>,
    denied: Option<&str>,
    note: Option<&str>,
) {
    let decision = match denied {
        Some(reason) => PolicyDecision::Deny { reason: reason.to_string(), policy_ids: vec!["project-isolation".into()] },
        None => PolicyDecision::Allow,
    };
    event_decision(principal, action, resource_type, resource_id, project, org, &decision, note);
}

/// [`event`] with any decision, e.g. an `Ask` an agent approval rule raised.
#[allow(clippy::too_many_arguments)]
pub fn event_decision(
    principal: &crate::Principal,
    action: &str,
    resource_type: &str,
    resource_id: &str,
    project: Option<&str>,
    org: Option<&str>,
    decision: &PolicyDecision,
    note: Option<&str>,
) {
    let Ok(action) = crate::Action::new(action) else { return };
    let mut resource = crate::Resource::new(resource_type, resource_id.to_string());
    if let Some(p) = project.filter(|p| !p.is_empty()) {
        resource.attributes.insert("namespace".into(), serde_json::Value::String(p.to_string()));
    }
    if let Some(o) = org.filter(|o| !o.is_empty()) {
        resource.attributes.insert(ORG_ATTR.into(), serde_json::Value::String(o.to_string()));
    }
    let req = ExecutionRequest {
        principal: principal.clone(),
        action,
        resource,
        context: Default::default(),
        attributes: Default::default(),
        params: serde_json::Value::Null,
    };
    record_noted(&req, decision, note);
}
