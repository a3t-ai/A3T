//! Agent keys: API keys for one agent's data plane.
//!
//! An app that calls an agent needs a token. A user's sign-in token is the
//! wrong thing to hand to an app, so each agent can have keys of its own:
//! `a3tk_<48 hex>`, made and revoked on its control plane
//! (`GET|POST /agents/{id}/keys`, `DELETE /agents/{id}/keys/{key}`).
//!
//! - A key is shown once, when it is made; only its SHA-256 is stored
//!   (`agent_keys`, platform DB), with its first characters to recognise it.
//! - A key calls that agent's data plane only: `POST /chat` with a session of
//!   that agent, and `GET /chat/stream/<that agent>:…`. Anything else is 403.
//! - It runs as the user who made it, with the roles they had then — never more,
//!   and only while they still reach its project (their memberships are checked
//!   on every call, as for any person); a user marked disabled takes their keys
//!   with them. A key can't be made while acting as someone (Act as).
//! - It expires (ID-5): 90 days unless asked (at most 365). Rotating makes its
//!   successor and lets the old key run out after a grace period, so two keys
//!   work while an app switches over.
//! - A blocked agent's keys are refused (RT-5), like any call to it.
//! - Revoking takes effect on the next call: every call reads the table.

use std::sync::{Arc, OnceLock};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::flowr_storage::StorageBackend;
use crate::handlers::call_lancedb::{query_platform_table, sql_quote, DEFAULT_PLATFORM_DB_PATH, PLATFORM_DB_RESOURCE};
use crate::platform_db::open_platform_db;

pub const TABLE: &str = "agent_keys";
const COLUMNS: [&str; 13] = ["id", "agent_id", "org_id", "name", "prefix", "hash", "roles", "created_by", "created_at", "last_used_at", "revoked_at", "expires_at", "rotated_from"];
/// A key's life unless asked, and the longest it may have (ID-5).
pub const DEFAULT_DAYS: i64 = 90;
pub const MAX_DAYS: i64 = 365;
pub const KEY_PREFIX: &str = "a3tk_";
/// How long a key's `last_used_at` may lag: one write per key per window.
const LAST_USED_EVERY_SECS: i64 = 300;

static STORE: OnceLock<Arc<dyn StorageBackend>> = OnceLock::new();

/// At startup, next to `access::install`: where keys are looked up.
/// The storage installed at start (None before `install`).
pub fn store() -> Option<Arc<dyn StorageBackend>> {
    STORE.get().cloned()
}

pub fn install(storage: Arc<dyn StorageBackend>) {
    let _ = STORE.set(storage);
}

/// What a request made with an agent key may reach: that agent's data plane.
#[derive(Debug, Clone)]
pub struct KeyScope {
    pub key_id: String,
    pub agent_id: String,
}

impl KeyScope {
    /// Whether this session (`<agent-id>:<conversation>`) is the key's agent's.
    pub fn owns_session(&self, session_id: &str) -> bool {
        session_id.split_once(':').map(|(a, rest)| a == self.agent_id && !rest.is_empty()).unwrap_or(false)
    }
}

pub fn is_key(token: &str) -> bool {
    token.starts_with(KEY_PREFIX)
}

/// The paths a key may call (the data plane). The agent is checked by the handler.
pub fn allowed(method: &axum::http::Method, path: &str) -> bool {
    (method == axum::http::Method::POST && path == "/chat")
        || (method == axum::http::Method::GET && path.starts_with("/chat/stream/"))
}

fn digest(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn col(row: &Value, k: &str) -> String {
    row.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

async fn ensure_table(storage: &dyn StorageBackend) -> Result<(), String> {
    let db = open_platform_db(storage, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let schema: Map<String, Value> = COLUMNS.iter().map(|c| (c.to_string(), json!("string"))).collect();
    db.create_table(TABLE, Some(&schema), "create_if_not_exists").await?;
    // ID-5: a table made before keys expired gains the columns; its keys get
    // the default life from now, so nothing stops at once. Asked of the backend
    // when it can say (LanceDB), else read from a row (ADB); an empty old table
    // is simply made again with the columns.
    // Once per process: a table seen with the columns stays that way.
    static CHECKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if CHECKED.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(());
    }
    let current = match db.columns(TABLE).await {
        Ok(Some(cols)) => Some(cols.iter().any(|c| c == "expires_at")),
        _ => match query_platform_table(storage, TABLE, None, 1).await {
            Ok(rows) => rows.first().map(|r| r.get("expires_at").is_some()),
            Err(_) => Some(true),
        },
    };
    if current != Some(true) {
        {
            let rows = query_platform_table(storage, TABLE, None, 100_000).await.unwrap_or_default();
            let until = (chrono::Utc::now() + chrono::Duration::days(DEFAULT_DAYS)).to_rfc3339();
            let rows: Vec<Value> = rows.into_iter().map(|mut r| {
                if let Some(m) = r.as_object_mut() {
                    m.insert("expires_at".into(), json!(until));
                    m.insert("rotated_from".into(), json!(""));
                }
                r
            }).collect();
            let columns: Vec<String> = COLUMNS.iter().map(|c| c.to_string()).collect();
            db.replace_table(TABLE, &columns, &rows).await?;
            tracing::info!(keys = rows.len(), "agent keys: expiry added; existing keys expire in {DEFAULT_DAYS} days");
        }
    }
    CHECKED.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// The principal and scope of a key, or why it is refused.
pub async fn verify(token: &str) -> Result<(flowr_policy::Principal, KeyScope), String> {
    let storage = STORE.get().ok_or("agent keys are not set up on this server")?;
    let rows = query_platform_table(storage.as_ref(), TABLE, Some(format!("hash = {}", sql_quote(&digest(token)))), 1)
        .await
        .map_err(|_| "unknown key".to_string())?;
    let row = rows.into_iter().next().ok_or("unknown key")?;
    if !col(&row, "revoked_at").is_empty() {
        return Err("this key was revoked".into());
    }
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(&col(&row, "expires_at")) {
        if t <= chrono::Utc::now() {
            return Err(format!("this key expired on {}", t.format("%Y-%m-%d %H:%M UTC")));
        }
    }
    // A key is its agent's: an agent that no longer exists takes every key with
    // it, however it was removed (deleting one revokes them too); a blocked
    // agent refuses them (RT-5).
    let agent = col(&row, "agent_id");
    let agents = query_platform_table(storage.as_ref(), "agent_configs", Some(format!("id = {}", sql_quote(&agent))), 1).await.unwrap_or_default();
    match agents.first() {
        None => return Err(format!("this key's agent ({agent}) was deleted")),
        Some(a) if col(a, "status") == "blocked" => return Err(format!("this key's agent ({agent}) is blocked")),
        _ => {}
    }
    // Its maker, as they are now: a disabled account takes its keys with it.
    let maker = col(&row, "created_by");
    if let Some(u) = query_platform_table(storage.as_ref(), "users", Some(format!("id = {}", sql_quote(&maker))), 1).await.ok().and_then(|r| r.into_iter().next()) {
        if matches!(col(&u, "status").as_str(), "disabled" | "deleted" | "suspended") {
            return Err("the person who made this key can no longer sign in".into());
        }
    }
    // RT-4: its agent's budget may limit calls a minute through each key.
    if let Some(n) = crate::budget::of(storage.as_ref(), &agent).await.and_then(|b| b.key_calls_per_minute) {
        crate::budget::key_call(&col(&row, "id"), n)?;
    }
    let roles: Vec<String> = col(&row, "roles").split(',').map(str::trim).filter(|r| !r.is_empty()).map(str::to_string).collect();
    let scope = KeyScope { key_id: col(&row, "id"), agent_id: col(&row, "agent_id") };
    // When it was last used, at most once per window (a write per call would be one per message).
    let last = chrono::DateTime::parse_from_rfc3339(&col(&row, "last_used_at")).map(|t| t.timestamp()).unwrap_or(0);
    let now = chrono::Utc::now();
    if now.timestamp() - last > LAST_USED_EVERY_SECS {
        let storage = Arc::clone(storage);
        let id = scope.key_id.clone();
        tokio::spawn(async move {
            let db = open_platform_db(storage.as_ref(), Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
            let mut v = Map::new();
            v.insert("last_used_at".into(), json!(now.to_rfc3339()));
            if let Err(e) = db.update(TABLE, &format!("id = {}", sql_quote(&id)), &v).await {
                tracing::warn!(key = %id, error = %e, "agent key: last_used_at not written");
            }
        });
    }
    Ok((flowr_policy::Principal::human(col(&row, "created_by"), roles), scope))
}

/// An agent's keys, newest first — never their hash.
pub async fn list(storage: &dyn StorageBackend, agent_id: &str) -> Result<Vec<Value>, String> {
    ensure_table(storage).await?;
    let mut rows = query_platform_table(storage, TABLE, Some(format!("agent_id = {}", sql_quote(agent_id))), 1000).await?;
    rows.sort_by(|a, b| col(b, "created_at").cmp(&col(a, "created_at")));
    Ok(rows.iter().map(|r| json!({
        "id": col(r, "id"), "name": col(r, "name"), "prefix": col(r, "prefix"),
        "created_by": col(r, "created_by"), "created_at": col(r, "created_at"),
        "last_used_at": col(r, "last_used_at"), "revoked_at": col(r, "revoked_at"),
        "expires_at": col(r, "expires_at"), "rotated_from": col(r, "rotated_from"),
    })).collect())
}

/// A new key for an agent: `{id, name, prefix, …, key}` — the only time the key is shown.
pub async fn create(storage: &dyn StorageBackend, agent: &Value, name: &str, by: &str, roles: &[String]) -> Result<Value, String> {
    create_for(storage, agent, name, by, roles, DEFAULT_DAYS, "").await
}

/// A new key that expires in `days` (1 to [`MAX_DAYS`]); `rotated_from`: the key it replaces.
pub async fn create_for(storage: &dyn StorageBackend, agent: &Value, name: &str, by: &str, roles: &[String], days: i64, rotated_from: &str) -> Result<Value, String> {
    if !(1..=MAX_DAYS).contains(&days) {
        return Err(format!("a key lives 1 to {MAX_DAYS} days"));
    }
    ensure_table(storage).await?;
    let secret = format!("{}{}{}", KEY_PREFIX, uuid::Uuid::new_v4().simple(), &uuid::Uuid::new_v4().simple().to_string()[..16]);
    let id = format!("ak_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let now = chrono::Utc::now().to_rfc3339();
    let row = json!({
        "id": id, "agent_id": col(agent, "id"), "org_id": col(agent, "org_id"),
        "name": if name.trim().is_empty() { "key".to_string() } else { name.trim().chars().take(80).collect() },
        "prefix": secret[..12].to_string(), "hash": digest(&secret), "roles": roles.join(","),
        "created_by": by, "created_at": now, "last_used_at": "", "revoked_at": "",
        "expires_at": (chrono::Utc::now() + chrono::Duration::days(days)).to_rfc3339(), "rotated_from": rotated_from,
    });
    let db = open_platform_db(storage, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    db.insert(TABLE, &[row.clone()]).await?;
    let mut out = row;
    if let Some(m) = out.as_object_mut() {
        m.remove("hash");
        m.remove("roles");
        m.insert("key".into(), json!(secret));
    }
    Ok(out)
}

/// Rotate a key (ID-5): a new key with its name and life, and the old one
/// runs out after `grace_hours` (or when it would have, if sooner). Returns the
/// new key (shown once) and when the old one stops.
pub async fn rotate(storage: &dyn StorageBackend, agent: &Value, key_id: &str, by: &str, roles: &[String], grace_hours: i64) -> Result<Option<(Value, String)>, String> {
    ensure_table(storage).await?;
    let filter = format!("id = {} AND agent_id = {}", sql_quote(key_id), sql_quote(&col(agent, "id")));
    let Some(old) = query_platform_table(storage, TABLE, Some(filter.clone()), 1).await?.into_iter().next() else { return Ok(None) };
    if !col(&old, "revoked_at").is_empty() {
        return Err("a revoked key isn't rotated; make a new one".into());
    }
    let days = chrono::DateTime::parse_from_rfc3339(&col(&old, "expires_at")).ok()
        .and_then(|e| chrono::DateTime::parse_from_rfc3339(&col(&old, "created_at")).ok().map(|c| (e - c).num_days()))
        .filter(|d| (1..=MAX_DAYS).contains(d)).unwrap_or(DEFAULT_DAYS);
    let new = create_for(storage, agent, &col(&old, "name"), by, roles, days, key_id).await?;
    let grace = chrono::Utc::now() + chrono::Duration::hours(grace_hours.clamp(0, 24 * 30));
    let ends = match chrono::DateTime::parse_from_rfc3339(&col(&old, "expires_at")) {
        Ok(t) if t.with_timezone(&chrono::Utc) < grace => t.with_timezone(&chrono::Utc),
        _ => grace,
    };
    let db = open_platform_db(storage, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let mut v = Map::new();
    v.insert("expires_at".into(), json!(ends.to_rfc3339()));
    db.update(TABLE, &filter, &v).await?;
    Ok(Some((new, ends.to_rfc3339())))
}

/// Revoke one of an agent's keys. Its row stays, to show when and that it was revoked.
pub async fn revoke(storage: &dyn StorageBackend, agent_id: &str, key_id: &str) -> Result<bool, String> {
    ensure_table(storage).await?;
    let filter = format!("id = {} AND agent_id = {}", sql_quote(key_id), sql_quote(agent_id));
    if query_platform_table(storage, TABLE, Some(filter.clone()), 1).await?.is_empty() {
        return Ok(false);
    }
    let db = open_platform_db(storage, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let mut v = Map::new();
    v.insert("revoked_at".into(), json!(chrono::Utc::now().to_rfc3339()));
    db.update(TABLE, &filter, &v).await?;
    Ok(true)
}

/// Revoke every key of these agents (they were deleted). Rows stay, like any revoked key.
pub async fn revoke_all(db: &dyn crate::platform_db::PlatformDb, agent_ids: &[String]) {
    let now = chrono::Utc::now().to_rfc3339();
    for id in agent_ids {
        let filter = format!("agent_id = {} AND revoked_at = ''", sql_quote(id));
        let mut v = Map::new();
        v.insert("revoked_at".into(), json!(now));
        match db.query(TABLE, Some(&filter), 1000).await {
            Ok(b) if crate::handlers::call_lancedb::batches_to_json(&b).is_empty() => {}
            Ok(_) => match db.update(TABLE, &filter, &v).await {
                Ok(()) => tracing::info!(agent = %id, "agent deleted: its keys are revoked"),
                Err(e) => tracing::warn!(agent = %id, error = %e, "agent deleted, but its keys were not revoked"),
            },
            // No keys table yet: no keys.
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_and_paths() {
        let s = KeyScope { key_id: "ak_1".into(), agent_id: "agt-1".into() };
        assert!(s.owns_session("agt-1:conv"));
        assert!(!s.owns_session("agt-2:conv"));
        assert!(!s.owns_session("agt-1:"));
        assert!(!s.owns_session("agt-1"));
        assert!(allowed(&axum::http::Method::POST, "/chat"));
        assert!(allowed(&axum::http::Method::GET, "/chat/stream/agt-1:x"));
        assert!(!allowed(&axum::http::Method::POST, "/message"));
        assert!(!allowed(&axum::http::Method::GET, "/agents/agt-1/keys"));
        assert!(is_key("a3tk_abc") && !is_key("eyJhbGc"));
        assert_eq!(digest("x").len(), 64);
    }
}
