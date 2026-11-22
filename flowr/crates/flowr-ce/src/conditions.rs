//! FlowR's [`ConditionSource`]: what rule conditions need beyond the call —
//! the org's time zone, the project's last deploy, personal data in a call's
//! arguments, and the counts A3T keeps for counted rules. Counts live in ADB
//! (platform table `policy_counts`; refusals in the decision log; tokens in
//! `agent_usage`), so they hold across every FlowR.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use flowr_policy::conditions::{ConditionSource, CountKey};
use flowr_storage::StorageBackend;
use serde_json::{json, Map, Value};
use tracing::warn;

use crate::handlers::call_lancedb::{query_platform_table, sql_quote, DEFAULT_PLATFORM_DB_PATH, PLATFORM_DB_RESOURCE};

pub const COUNTS: &str = "policy_counts";
const COUNT_COLUMNS: [&str; 4] = ["rule", "subject", "at_ms", "id"];
const FRESH: Duration = Duration::from_secs(60);

pub struct Conditions {
    storage: Arc<dyn StorageBackend>,
    /// namespace → (when read, org slug, time zone).
    zones: Mutex<HashMap<String, (Instant, Option<String>, Option<String>)>>,
}

pub fn install(storage: Arc<dyn StorageBackend>) {
    flowr_policy::conditions::set_condition_source(Arc::new(Conditions { storage, zones: Mutex::new(HashMap::new()) }));
}

fn col(r: &Value, k: &str) -> String {
    r.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Conditions {
    /// The org (slug) a project belongs to, and the org's time zone.
    async fn org_of(&self, ns: &str) -> (Option<String>, Option<String>) {
        if let Some((at, org, tz)) = self.zones.lock().unwrap().get(ns).cloned() {
            if at.elapsed() < FRESH {
                return (org, tz);
            }
        }
        let s = self.storage.as_ref();
        let org = query_platform_table(s, "projects", Some(format!("namespace = {} AND owner_kind = 'org'", sql_quote(ns))), 1).await.ok()
            .and_then(|r| r.into_iter().next()).map(|r| col(&r, "owner")).filter(|o| !o.is_empty());
        let tz = match &org {
            Some(o) => crate::policy_changes::governance(s, o).await.timezone,
            None => None,
        };
        self.zones.lock().unwrap().insert(ns.to_string(), (Instant::now(), org.clone(), tz.clone()));
        (org, tz)
    }

    async fn db(&self) -> Result<Arc<dyn crate::platform_db::PlatformDb>, String> {
        let db = crate::platform_db::open_platform_db(self.storage.as_ref(), Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
        let schema: Map<String, Value> = COUNT_COLUMNS.iter().map(|c| (c.to_string(), json!("string"))).collect();
        db.create_table(COUNTS, Some(&schema), "create_if_not_exists").await?;
        Ok(db)
    }

    /// Who a count is about, as the rule says (`by`).
    async fn subject(&self, spec: &Value, key: &CountKey) -> Option<String> {
        let by = spec.get("by").and_then(|v| v.as_str()).unwrap_or("person");
        match by {
            "person" => key.person.clone().or_else(|| key.agent.clone()).map(|p| format!("person:{p}")),
            "agent" => key.agent.clone().or_else(|| key.person.clone()).map(|a| format!("agent:{a}")),
            "project" => key.namespace.clone().map(|n| format!("project:{n}")),
            "org" => match &key.namespace {
                Some(ns) => self.org_of(ns).await.0.map(|o| format!("org:{o}")),
                None => None,
            },
            _ => None,
        }
    }

    async fn count_since(&self, rule: &str, subject: &str, since_ms: i64, max: usize) -> Result<usize, String> {
        let filter = format!("rule = {} AND subject = {} AND at_ms >= {}", sql_quote(rule), sql_quote(subject), sql_quote(&format!("{since_ms:015}")));
        self.db().await?;
        query_platform_table(self.storage.as_ref(), COUNTS, Some(filter), max).await.map(|r| r.len())
    }

    /// Refusals of a person (theirs, and their agents') in the decision log.
    async fn refusal_times(&self, key: &CountKey, since_ms: u64) -> Result<Vec<u64>, String> {
        let Some(person) = key.person.clone() else { return Ok(Vec::new()) };
        let org = match &key.namespace {
            Some(ns) => self.org_of(ns).await.0,
            None => None,
        };
        // The log is kept per org id (`_audit.<org id>`).
        let org_id = match org {
            Some(slug) => crate::org_policy::org_id(self.storage.as_ref(), &slug).await,
            None => None,
        };
        let orgs = vec![org_id.unwrap_or_else(|| "platform".into())];
        let mut out = Vec::new();
        for q in [
            crate::decision_log::Query { orgs: orgs.clone(), principal: Some(person.clone()), decision: Some("deny".into()), since_ms: Some(since_ms), limit: 500, ..Default::default() },
            crate::decision_log::Query { orgs: orgs.clone(), on_behalf_of: Some(person.clone()), decision: Some("deny".into()), since_ms: Some(since_ms), limit: 500, ..Default::default() },
        ] {
            let (rows, _) = crate::decision_log::recall(&q).await?;
            for r in rows {
                // A lockout's own refusals don't keep it going.
                let ids: Vec<String> = r.get("policy_ids").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                if ids.iter().any(|i| i.starts_with(&key.rule)) {
                    continue;
                }
                if let Some(t) = r.get("ts_ms").and_then(|t| t.as_u64()) {
                    out.push(t);
                }
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }
}

/// Locked out now: at some moment in the last `block` minutes, `n` refusals
/// had come within `mins` minutes. (`times` sorted, ms.)
pub fn locked_out(times: &[u64], now_ms: u64, n: usize, mins: u64, block: u64) -> bool {
    if n == 0 || times.len() < n {
        return false;
    }
    let (win, blk) = (mins * 60_000, block * 60_000);
    // The n-th refusal of a run of n within `win` starts the lockout.
    times.windows(n).any(|w| w[n - 1] - w[0] <= win && now_ms.saturating_sub(w[n - 1]) < blk)
}

#[async_trait]
impl ConditionSource for Conditions {
    async fn utc_offset_minutes(&self, namespace: Option<&str>) -> i32 {
        use chrono::Offset;
        let Some(ns) = namespace else { return 0 };
        let Some(tz) = self.org_of(ns).await.1 else { return 0 };
        match tz.parse::<chrono_tz::Tz>() {
            Ok(z) => chrono::Utc::now().with_timezone(&z).offset().fix().local_minus_utc() / 60,
            Err(_) => 0,
        }
    }

    async fn since_deploy_s(&self, namespace: Option<&str>) -> Option<i64> {
        let at = crate::ops::last_deploy_in(namespace?)?;
        Some((chrono::Utc::now() - at).num_seconds().max(0))
    }

    fn args_pii(&self, params: &Value) -> bool {
        let (_, kinds) = crate::pii::redact(&params.to_string());
        kinds.iter().any(|k| *k != "secret")
    }

    async fn over(&self, spec: &Value, key: &CountKey) -> Result<bool, String> {
        let n = |k: &str| spec.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
        match spec.get("kind").and_then(|v| v.as_str()) {
            Some("rate") => {
                let window: i64 = match spec.get("per").and_then(|v| v.as_str()) { Some("minute") => 60, Some("day") => 86_400, _ => 3600 };
                let Some(subject) = self.subject(spec, key).await else { return Ok(false) };
                let limit = n("n") as usize;
                Ok(self.count_since(&key.rule, &subject, now_ms() - window * 1000, limit + 1).await? >= limit)
            }
            Some("first_use") => {
                let who = key.person.clone().or_else(|| key.agent.clone()).unwrap_or_default();
                Ok(self.count_since(&key.rule, &format!("first:{who}|{}", key.resource), 0, 1).await? == 0)
            }
            Some("refusals") => {
                let (count, mins, block) = (n("n") as usize, n("mins"), n("block"));
                let now = now_ms() as u64;
                let times = self.refusal_times(key, now.saturating_sub((mins + block) * 60_000)).await?;
                Ok(locked_out(&times, now, count, mins, block))
            }
            Some("tokens") => {
                let Some(agent) = key.agent.as_deref() else { return Ok(false) };
                let agent = agent.split(':').nth(1).unwrap_or(agent);
                let (_, tokens) = crate::budget::used_today(self.storage.as_ref(), agent).await;
                Ok(tokens > n("n"))
            }
            other => Err(format!("unknown counted condition {other:?}")),
        }
    }

    async fn record(&self, spec: &Value, key: &CountKey) {
        let subject = match spec.get("kind").and_then(|v| v.as_str()) {
            Some("rate") => self.subject(spec, key).await,
            Some("first_use") => Some(format!("first:{}|{}", key.person.clone().or_else(|| key.agent.clone()).unwrap_or_default(), key.resource)),
            _ => None,
        };
        let Some(subject) = subject else { return };
        let row = json!({ "rule": key.rule, "subject": subject, "at_ms": format!("{:015}", now_ms()), "id": uuid::Uuid::new_v4().simple().to_string() });
        match self.db().await {
            Ok(db) => {
                if let Err(e) = db.insert(COUNTS, &[row]).await {
                    warn!(error = %e, rule = %key.rule, "policy count not written");
                }
            }
            Err(e) => warn!(error = %e, "policy counts table unavailable"),
        }
    }
}

/// Drop rate counts older than a day (first uses are kept). Hourly.
pub fn spawn_sweep(storage: Arc<dyn StorageBackend>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            let db = crate::platform_db::open_platform_db(storage.as_ref(), Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
            let cut = format!("{:015}", now_ms() - 86_400_000);
            if let Err(e) = db.delete(COUNTS, &format!("at_ms < {} AND subject NOT LIKE 'first:%'", sql_quote(&cut))).await {
                warn!(error = %e, "policy counts: old rows not swept");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn lockouts() {
        let m = 60_000;
        // Three refusals within 10 minutes, the last 5 minutes ago: locked for 30.
        assert!(super::locked_out(&[0, 2 * m, 4 * m], 9 * m, 3, 10, 30));
        // ... but not 35 minutes after.
        assert!(!super::locked_out(&[0, 2 * m, 4 * m], 39 * m, 3, 10, 30));
        // Spread over 20 minutes: no run of 3 within 10.
        assert!(!super::locked_out(&[0, 10 * m + 1, 20 * m + 2], 21 * m, 3, 10, 30));
        assert!(!super::locked_out(&[0, m], 2 * m, 3, 10, 30));
    }
}
