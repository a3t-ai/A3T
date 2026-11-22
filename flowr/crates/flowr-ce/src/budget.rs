//! Agent budgets and key rate limits (RT-4).
//!
//! An agent's own policy may set `budget: {runs_per_day, tokens_per_day,
//! key_calls_per_minute}`. Before each turn its use today (UTC) is read from
//! `agent_usage` (one row per turn: its model tokens); at or over a limit the
//! turn is refused — a hard stop until the next day — and `budget:exceeded` goes
//! to the decision log once a day per agent and limit. Keys are rate limited per
//! minute across every FlowR: each call through a key is one row of
//! `agent_key_calls`, counted for the minute after it is written (two calls that
//! race can both be refused at the limit, never both let past it).

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

pub const KEY_CALLS: &str = "agent_key_calls";

/// A key's calls this minute, in this process: `Err` past `per_minute` (the
/// quick refusal before the cluster-wide count).
fn local_call(key_id: &str, per_minute: u64) -> Result<(), String> {
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

/// One call through key `key_id`: `Err` past `per_minute` calls this minute,
/// counted across every FlowR.
pub async fn key_call(s: &dyn StorageBackend, key_id: &str, per_minute: u64) -> Result<(), String> {
    local_call(key_id, per_minute)?;
    use crate::handlers::call_lancedb::{DEFAULT_PLATFORM_DB_PATH, PLATFORM_DB_RESOURCE};
    static READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static SWEPT: Mutex<Option<Instant>> = Mutex::new(None);
    let minute = (chrono::Utc::now().timestamp() / 60).to_string();
    let db = crate::platform_db::open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    if !READY.load(std::sync::atomic::Ordering::Relaxed) {
        let schema: serde_json::Map<String, serde_json::Value> = ["id", "key_id", "minute"].iter().map(|c| (c.to_string(), serde_json::json!("string"))).collect();
        if db.create_table(KEY_CALLS, Some(&schema), "create_if_not_exists").await.is_ok() {
            READY.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let row = serde_json::json!({ "id": uuid::Uuid::now_v7().to_string(), "key_id": key_id, "minute": minute });
    if let Err(e) = db.insert(KEY_CALLS, &[row]).await {
        // The shared count is down: this process's count still holds.
        tracing::warn!(error = %e, "key rate: shared count not written; this FlowR's own count applies");
        return Ok(());
    }
    let n = query_platform_table(s, KEY_CALLS, Some(format!("key_id = {} AND minute = {}", sql_quote(key_id), sql_quote(&minute))), (per_minute + 1) as usize)
        .await.map(|r| r.len() as u64).unwrap_or(0);
    // Old minutes go, from one call every ten minutes per FlowR.
    let sweep = {
        let mut last = SWEPT.lock().unwrap();
        let due = last.is_none_or(|t| t.elapsed() > Duration::from_secs(600));
        if due { *last = Some(Instant::now()); }
        due
    };
    if sweep {
        let before = (chrono::Utc::now().timestamp() / 60 - 5).to_string();
        let _ = db.delete(KEY_CALLS, &format!("minute < {}", sql_quote(&before))).await;
    }
    if n > per_minute {
        return Err(format!("this key may make {per_minute} calls a minute"));
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
        for _ in 0..5 { assert!(local_call("k-test", 5).is_ok()); }
        assert!(local_call("k-test", 5).is_err());
        assert!(local_call("k-other", 5).is_ok());
    }
}
