//! Agent budgets and key rate limits (RT-4).
//!
//! An agent's own policy may set `budget: {runs_per_day, tokens_per_day,
//! key_calls_per_minute}`. Before each turn its use today (UTC) is read from
//! `agent_usage` (one row per turn: its model tokens); at or over a limit the
//! turn is refused — a hard stop until the next day — and `budget:exceeded` goes
//! to the decision log once a day per agent and limit. Keys are rate limited per
//! minute in each FlowR process (so the cluster-wide ceiling is that times the
//! number of FlowRs serving the data plane).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use flowr_policy::policy_file::Budget;
use flowr_storage::StorageBackend;

use crate::handlers::call_lancedb::{query_platform_table, sql_quote};

const FRESH: Duration = Duration::from_secs(20);

/// The agent's budget, from its own policy (cached briefly).
pub async fn of(s: &dyn StorageBackend, agent: &str) -> Option<Budget> {
    static CACHE: Mutex<Option<HashMap<String, (Instant, Option<Budget>)>>> = Mutex::new(None);
    if let Some((at, b)) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(agent).cloned()) {
        if at.elapsed() < FRESH {
            return b;
        }
    }
    let b = query_platform_table(s, crate::agent_space::security::TABLE, Some(format!("agent_id = {}", sql_quote(agent))), 1).await.ok()
        .and_then(|r| r.into_iter().next())
        .and_then(|r| r.get("policy").and_then(|v| v.as_str()).map(str::to_string))
        .and_then(|y| flowr_policy::policy_file::PolicyFile::parse(&y).ok())
        .and_then(|f| f.budget);
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(agent.to_string(), (Instant::now(), b.clone()));
    b
}

/// Today's use: (turns, tokens).
pub async fn used_today(s: &dyn StorageBackend, agent: &str) -> (u64, u64) {
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let rows = query_platform_table(s, crate::agent_space::USAGE, Some(format!("agent_id = {} AND at >= {}", sql_quote(agent), sql_quote(&day))), 100_000)
        .await.unwrap_or_default();
    let n = |r: &serde_json::Value, k: &str| r.get(k).and_then(|v| v.as_str()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    (rows.len() as u64, rows.iter().map(|r| n(r, "prompt_tokens") + n(r, "completion_tokens")).sum())
}

/// What is over, if anything: the reason a turn is refused.
pub fn over(b: &Budget, used: (u64, u64)) -> Option<String> {
    if let Some(max) = b.runs_per_day.filter(|m| used.0 >= *m) {
        return Some(format!("it has used its {max} turns for today"));
    }
    if let Some(max) = b.tokens_per_day.filter(|m| used.1 >= *m) {
        return Some(format!("it has used its {max} model tokens for today ({} so far)", used.1));
    }
    None
}

/// May agent `agent` take a turn now? `Err`: why not (and the alert is sent).
pub async fn admit(s: &dyn StorageBackend, agent: &str) -> Result<(), String> {
    let Some(b) = of(s, agent).await else { return Ok(()) };
    if b.runs_per_day.is_none() && b.tokens_per_day.is_none() {
        return Ok(());
    }
    let used = used_today(s, agent).await;
    match over(&b, used) {
        None => Ok(()),
        Some(why) => {
            alert_once(agent, &why);
            Err(format!("{agent} is over its budget: {why}. It runs again tomorrow (UTC), or when its budget is raised."))
        }
    }
}

/// One `budget:exceeded` per agent and reason a day.
fn alert_once(agent: &str, why: &str) {
    static SENT: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let key = format!("{agent}|{}", why.split(" (").next().unwrap_or(why));
    let mut sent = SENT.lock().unwrap();
    let m = sent.get_or_insert_with(HashMap::new);
    if m.get(&key) == Some(&day) {
        return;
    }
    m.insert(key, day);
    tracing::warn!(agent, reason = why, "agent over budget: stopped until tomorrow");
    let who = flowr_policy::Principal::service("budget");
    flowr_policy::audit::event_decision(&who, "budget:exceeded", "Agent", agent, None, None,
        &flowr_policy::PolicyDecision::Deny { reason: why.to_string(), policy_ids: vec!["budget".into()] }, Some("stopped until tomorrow (UTC)"));
}

/// A key's calls this minute, in this process: `Err` past `per_minute`.
pub fn key_call(key_id: &str, per_minute: u64) -> Result<(), String> {
    static WINDOW: Mutex<Option<HashMap<String, (u64, u64)>>> = Mutex::new(None);
    let minute = (chrono::Utc::now().timestamp() / 60) as u64;
    let mut w = WINDOW.lock().unwrap();
    let m = w.get_or_insert_with(HashMap::new);
    let e = m.entry(key_id.to_string()).or_insert((minute, 0));
    if e.0 != minute {
        *e = (minute, 0);
    }
    e.1 += 1;
    if e.1 > per_minute {
        return Err(format!("this key may make {per_minute} calls a minute"));
    }
    if m.len() > 50_000 {
        m.retain(|_, v| v.0 == minute);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_stop_at_the_line() {
        let b = Budget { runs_per_day: Some(3), tokens_per_day: Some(1000), key_calls_per_minute: None };
        assert!(over(&b, (2, 999)).is_none());
        assert!(over(&b, (3, 0)).unwrap().contains("turns"));
        assert!(over(&b, (1, 1000)).unwrap().contains("tokens"));
        assert!(over(&Budget::default(), (1_000_000, 1_000_000_000)).is_none());
    }

    #[test]
    fn keys_are_limited_per_minute() {
        for _ in 0..5 { assert!(key_call("k-test", 5).is_ok()); }
        assert!(key_call("k-test", 5).is_err());
        assert!(key_call("k-other", 5).is_ok());
    }
}
