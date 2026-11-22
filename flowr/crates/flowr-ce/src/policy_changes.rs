//! A second approver for policy changes (GOV-3, "four eyes"), per org.
//!
//! With `four_eyes` on for an org, a change to its policy files (save, remove,
//! switch on or off) or to one of its agents' own policies is not applied: it
//! is checked as usual — a change that wouldn't compile is refused at once —
//! and then held as a pending change until another of the org's admins or
//! security admins (or platform InfoSec) approves it. The author can't approve
//! their own change. Approving re-checks it against the files as they are then
//! and applies it under the author's name; the history and the decision log
//! name both. Turning `four_eyes` off is itself a change a second person approves.
//!
//! - `GET  /security/orgs/{org}/settings` · `PUT {four_eyes}`
//! - `GET  /security/orgs/{org}/changes`: pending first, then the latest decided
//! - `POST /security/orgs/{org}/changes/{id}/approve` · `/reject` `{note?}`
//!
//! Decision log: `policy:propose`, `policy:approve`, `policy:reject`.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use serde_json::{json, Map, Value};

use flowr_policy::Principal;
use flowr_storage::StorageBackend;

use crate::handlers::call_lancedb::{query_platform_table, sql_quote, DEFAULT_PLATFORM_DB_PATH, PLATFORM_DB_RESOURCE};
use crate::platform_db::{open_platform_db, PlatformDb};

pub const SETTINGS: &str = "org_policy_settings";
const SETTINGS_COLUMNS: [&str; 4] = ["org", "four_eyes", "updated_by", "updated_at"];
/// Who governs an org (governance round 2): who may change its policies and its
/// identity (org admins and security admins, or security admins only — then
/// admins propose), who approves a held change, and the org's time zone.
pub const GOVERNANCE: &str = "org_governance";
const GOVERNANCE_COLUMNS: [&str; 7] = ["org", "policy_lock", "identity_lock", "approver", "timezone", "updated_by", "updated_at"];
pub const CHANGES: &str = "policy_changes";
const CHANGE_COLUMNS: [&str; 13] = [
    "id", "org", "target", "op", "subject", "payload", "summary", "by", "at", "status", "decided_by", "decided_at", "note",
];

type Reply = Result<Response, Response>;
type St = State<Arc<dyn StorageBackend>>;
type P = Option<Extension<Principal>>;

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}
fn col(row: &Value, k: &str) -> String {
    row.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}
fn who(p: P) -> Principal {
    p.map(|Extension(p)| p).unwrap_or_else(|| Principal::service("anonymous"))
}
fn internal(e: impl ToString) -> Response {
    err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub fn routes() -> Router<Arc<dyn StorageBackend>> {
    Router::new()
        .route("/security/orgs/:org/settings", get(get_settings).put(put_settings))
        .route("/security/orgs/:org/changes", get(list_changes))
        .route("/security/orgs/:org/changes/:id/approve", post(approve))
        .route("/security/orgs/:org/changes/:id/reject", post(reject))
}

async fn db(s: &dyn StorageBackend) -> Result<Arc<dyn PlatformDb>, String> {
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    for (t, cols) in [(SETTINGS, &SETTINGS_COLUMNS[..]), (CHANGES, &CHANGE_COLUMNS[..]), (GOVERNANCE, &GOVERNANCE_COLUMNS[..])] {
        let schema: Map<String, Value> = cols.iter().map(|c| (c.to_string(), json!("string"))).collect();
        db.create_table(t, Some(&schema), "create_if_not_exists").await?;
    }
    Ok(db)
}

/// Does `org` (its slug) need a second approver for policy changes?
pub async fn four_eyes(s: &dyn StorageBackend, org: &str) -> bool {
    query_platform_table(s, SETTINGS, Some(format!("org = {}", sql_quote(org))), 1).await
        .ok().and_then(|r| r.into_iter().next()).map(|r| col(&r, "four_eyes") == "true").unwrap_or(false)
}

/// Make the settings tables at start: the org flow reads `org_governance`.
pub async fn ensure(s: &dyn StorageBackend) {
    if let Err(e) = db(s).await {
        tracing::warn!(error = %e, "org settings tables not made; retried on first use");
    }
}

/// An org's governance settings (defaults: both may change, a security admin approves, UTC).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Governance {
    /// `both` (org admins and security admins) or `security` (security admins only).
    pub policy_lock: String,
    pub identity_lock: String,
    /// Who approves a held change: `securityadmin`, or `admin` (an admin or a security admin).
    pub approver: String,
    /// IANA time zone for rule conditions (hours, dates); `None`: UTC.
    pub timezone: Option<String>,
}

impl Default for Governance {
    fn default() -> Self {
        Governance { policy_lock: "both".into(), identity_lock: "both".into(), approver: "securityadmin".into(), timezone: None }
    }
}

pub async fn governance(s: &dyn StorageBackend, org: &str) -> Governance {
    let Some(r) = query_platform_table(s, GOVERNANCE, Some(format!("org = {}", sql_quote(org))), 1).await.ok().and_then(|r| r.into_iter().next()) else {
        return Governance::default();
    };
    let or = |k: &str, d: &str| Some(col(&r, k)).filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string());
    Governance {
        policy_lock: or("policy_lock", "both"),
        identity_lock: or("identity_lock", "both"),
        approver: or("approver", "securityadmin"),
        timezone: Some(col(&r, "timezone")).filter(|v| !v.is_empty()),
    }
}

/// The role `principal` holds for `org` (slug): `platform` (SuperAdmin or
/// SecurityAdmin), `securityadmin`, `admin`, `member`, or empty.
pub async fn org_role(s: &dyn StorageBackend, principal: &Principal, org: &str) -> String {
    if principal.roles().iter().any(|r| r == flowr_policy::roles::SUPER_ADMIN || r == flowr_policy::roles::SECURITY_ADMIN) {
        return "platform".into();
    }
    let Some(user) = flowr_policy::namespace::acting_user(principal) else { return String::new() };
    let Some(org_id) = crate::org_policy::org_id(s, org).await else { return String::new() };
    query_platform_table(s, "org_members", Some(format!("org_id = {} AND user_id = {}", sql_quote(&org_id), sql_quote(user))), 1).await
        .ok().and_then(|r| r.into_iter().next()).map(|r| col(&r, "role")).unwrap_or_default()
}

/// Is a change by `principal` held for approval because the org lets only its
/// security admins change its policies (or, `identity`, its identity)?
pub async fn held_by_lock(s: &dyn StorageBackend, principal: &Principal, org: &str, identity: bool) -> bool {
    let g = governance(s, org).await;
    let lock = if identity { &g.identity_lock } else { &g.policy_lock };
    lock == "security" && org_role(s, principal, org).await == "admin"
}

/// May `principal` approve `org`'s held changes (as the org says)?
pub async fn may_approve(s: &dyn StorageBackend, principal: &Principal, org: &str) -> Result<(), Response> {
    let role = org_role(s, principal, org).await;
    let g = governance(s, org).await;
    match role.as_str() {
        "platform" | "securityadmin" => Ok(()),
        "admin" if g.approver == "admin" => Ok(()),
        _ => Err(err(StatusCode::FORBIDDEN, format!("{org}'s changes are approved by {}", if g.approver == "admin" { "an org admin or security admin" } else { "an org security admin" }))),
    }
}

/// GOV-9: an update to an agent's record, held for a second approver when its
/// org has four_eyes on. `Some(answer)` when held (the flow says so); `None`:
/// apply it now.
pub async fn hold_agent_update(s: &dyn StorageBackend, principal: &Principal, row: &Value, filter: &str, values: &Map<String, Value>) -> Option<Value> {
    let org_id = col(row, "org_id");
    let slug = query_platform_table(s, "orgs", Some(format!("id = {}", sql_quote(&org_id))), 1).await.ok()
        .and_then(|r| r.into_iter().next()).map(|r| col(&r, "slug")).filter(|x| !x.is_empty())?;
    if !four_eyes(s, &slug).await {
        return None;
    }
    let id = col(row, "id");
    let resp = propose(s, principal, &slug, "agent-record", "update", &id, json!({ "filter": filter, "values": values }),
        &format!("change agent {}'s record", col(row, "name"))).await;
    let change = match resp {
        Ok(_) => query_platform_table(s, CHANGES, Some(format!("org = {} AND subject = {} AND status = 'pending'", sql_quote(&slug), sql_quote(&id))), 1000).await.ok()
            .and_then(|mut r| { r.sort_by(|a, b| col(b, "at").cmp(&col(a, "at"))); r.into_iter().next() }).map(|r| col(&r, "id")),
        Err(_) => None,
    };
    Some(json!({ "updated": false, "held": true, "change": change,
        "message": format!("{slug} needs a second approver for changes to its agents: another admin or security admin approves it in Security › {slug} › Policy files › Changes.") }))
}

/// Apply an approved agent-record update.
async fn apply_agent_update(s: &dyn StorageBackend, principal: &Principal, payload: &Value, author: &str) -> Reply {
    let filter = payload["filter"].as_str().unwrap_or_default();
    let values = payload["values"].as_object().cloned().unwrap_or_default();
    if filter.is_empty() || values.is_empty() {
        return Err(err(StatusCode::UNPROCESSABLE_ENTITY, "the held change has no filter or values"));
    }
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let before = query_platform_table(s, "agent_configs", Some(filter.to_string()), 1).await.ok().and_then(|r| r.into_iter().next());
    db.update("agent_configs", filter, &values).await.map_err(internal)?;
    if let Some(b) = before {
        let changed: Vec<&String> = values.keys().filter(|k| b.get(*k) != values.get(*k)).collect();
        let _ = db.insert("agent_events", &[json!({
            "id": format!("{}-evt-{}", col(&b, "id"), chrono::Utc::now().timestamp_micros()), "agent_config_id": col(&b, "id"), "event_type": "updated",
            "payload": json!({ "by": author, "approved_by": principal.id(), "changed": changed,
                "before": changed.iter().map(|k| (k.to_string(), b.get(*k).cloned().unwrap_or(Value::Null))).collect::<Map<String, Value>>(),
                "after": changed.iter().map(|k| (k.to_string(), values.get(*k).cloned().unwrap_or(Value::Null))).collect::<Map<String, Value>>() }).to_string(),
            "created_at": chrono::Utc::now().to_rfc3339(),
        })]).await;
    }
    Ok(Json(json!({ "updated": true })).into_response())
}

/// Hold a change for a second approver. The reply says so (202).
#[allow(clippy::too_many_arguments)]
pub async fn propose(s: &dyn StorageBackend, principal: &Principal, org: &str, target: &str, op: &str, subject: &str, payload: Value, summary: &str) -> Reply {
    let db = db(s).await.map_err(internal)?;
    let id = uuid::Uuid::now_v7().to_string();
    let at = chrono::Utc::now().to_rfc3339();
    db.insert(CHANGES, &[json!({
        "id": id, "org": org, "target": target, "op": op, "subject": subject, "payload": payload.to_string(),
        "summary": summary, "by": principal.id(), "at": at, "status": "pending", "decided_by": "", "decided_at": "", "note": "",
    })]).await.map_err(internal)?;
    flowr_policy::audit::event(principal, "policy:propose", "PolicyChange", &id, None, Some(org), None, Some(summary));
    tracing::info!(org, %id, summary, by = %principal.id(), "policy change held for a second approver");
    Ok((StatusCode::ACCEPTED, Json(json!({
        "pending": true, "change": id, "org": org, "summary": summary,
        "message": format!("{org} needs a second approver for policy changes: another admin or security admin approves it in Security › {org} › Policy files › Changes."),
    }))).into_response())
}

async fn get_settings(State(s): St, p: P, Path(org): Path<String>) -> Response {
    let principal = who(p);
    let s = s.as_ref();
    if let Err(r) = crate::org_policy::allowed(s, &principal, &org).await {
        return r;
    }
    let pending = query_platform_table(s, CHANGES, Some(format!("org = {} AND status = 'pending'", sql_quote(&org))), 1000).await
        .map(|r| r.len()).unwrap_or(0);
    let role = org_role(s, &principal, &org).await;
    Json(json!({ "org": org, "four_eyes": four_eyes(s, &org).await, "pending": pending,
        "governance": governance(s, &org).await,
        // What the caller may do here: change this page, and approve.
        "my_role": role, "can_govern": role == "platform" || role == "securityadmin",
        "can_approve": may_approve(s, &principal, &org).await.is_ok() })).into_response()
}

async fn put_settings(State(s): St, p: P, Path(org): Path<String>, Json(b): Json<Value>) -> Response {
    let principal = who(p);
    let s = s.as_ref();
    if let Err(r) = crate::org_policy::allowed(s, &principal, &org).await {
        return r;
    }
    // Who governs the org is decided by its security admins (and the platform's).
    let role = org_role(s, &principal, &org).await;
    if role != "platform" && role != "securityadmin" {
        return err(StatusCode::FORBIDDEN, format!("{org}'s security admins change these settings; you can read them"));
    }
    let gov_keys = ["policy_lock", "identity_lock", "approver", "timezone"];
    if gov_keys.iter().any(|k| b.get(*k).is_some()) {
        if let Err(r) = set_governance(s, &principal, &org, &b).await {
            return r;
        }
    }
    match b.get("four_eyes").and_then(|v| v.as_bool()) {
        Some(on) => match set_four_eyes(s, &principal, &org, on, None).await { Ok(r) | Err(r) => r },
        None if gov_keys.iter().any(|k| b.get(*k).is_some()) => Json(json!({ "org": org, "governance": governance(s, &org).await })).into_response(),
        None => err(StatusCode::BAD_REQUEST, "give four_eyes, policy_lock, identity_lock, approver or timezone"),
    }
}

async fn set_governance(s: &dyn StorageBackend, principal: &Principal, org: &str, b: &Value) -> Result<(), Response> {
    let mut g = governance(s, org).await;
    let pick = |k: &str, allowed: &[&str], cur: &mut String| -> Result<(), Response> {
        if let Some(v) = b.get(k) {
            let v = v.as_str().unwrap_or_default();
            if !allowed.contains(&v) {
                return Err(err(StatusCode::BAD_REQUEST, format!("{k}: one of {}", allowed.join(", "))));
            }
            *cur = v.to_string();
        }
        Ok(())
    };
    pick("policy_lock", &["both", "security"], &mut g.policy_lock)?;
    pick("identity_lock", &["both", "security"], &mut g.identity_lock)?;
    pick("approver", &["securityadmin", "admin"], &mut g.approver)?;
    if let Some(v) = b.get("timezone") {
        let tz = v.as_str().unwrap_or_default().trim();
        if tz.is_empty() || tz == "UTC" {
            g.timezone = None;
        } else if tz.parse::<chrono_tz::Tz>().is_ok() {
            g.timezone = Some(tz.to_string());
        } else {
            return Err(err(StatusCode::BAD_REQUEST, format!("timezone {tz:?}: an IANA name such as Europe/Berlin")));
        }
    }
    let db = db(s).await.map_err(internal)?;
    db.upsert(GOVERNANCE, &["org"], &[json!({
        "org": org, "policy_lock": g.policy_lock, "identity_lock": g.identity_lock, "approver": g.approver,
        "timezone": g.timezone.clone().unwrap_or_default(), "updated_by": principal.id(), "updated_at": chrono::Utc::now().to_rfc3339(),
    })]).await.map_err(internal)?;
    flowr_policy::audit::event(principal, "policy:settings", "Org", org, None, Some(org), None,
        Some(&format!("policies: {} · identity: {} · approver: {} · time zone: {}", g.policy_lock, g.identity_lock, g.approver, g.timezone.as_deref().unwrap_or("UTC"))));
    Ok(())
}

/// Switch the second approver. Turning it off, while it is on, is a change a
/// second person approves; turning it on applies at once.
async fn set_four_eyes(s: &dyn StorageBackend, principal: &Principal, org: &str, on: bool, approved: Option<&str>) -> Reply {
    crate::org_policy::allowed(s, principal, org).await?;
    let now_on = four_eyes(s, org).await;
    if now_on == on {
        return Ok(Json(json!({ "org": org, "four_eyes": on, "changed": false })).into_response());
    }
    if now_on && !on && approved.is_none() {
        return propose(s, principal, org, "settings", "four_eyes", "", json!({ "four_eyes": false }), "turn off the second approver").await;
    }
    let by = approved.map(str::to_string).unwrap_or_else(|| principal.id().to_string());
    let db = db(s).await.map_err(internal)?;
    db.upsert(SETTINGS, &["org"], &[json!({
        "org": org, "four_eyes": on.to_string(), "updated_by": by, "updated_at": chrono::Utc::now().to_rfc3339(),
    })]).await.map_err(internal)?;
    flowr_policy::audit::event(principal, "policy:settings", "Org", org, None, Some(org), None,
        Some(if on { "second approver on" } else { "second approver off" }));
    Ok(Json(json!({ "org": org, "four_eyes": on, "changed": true })).into_response())
}

fn change_json(r: &Value) -> Value {
    json!({
        "id": col(r, "id"), "target": col(r, "target"), "op": col(r, "op"), "subject": col(r, "subject"),
        "summary": col(r, "summary"), "by": col(r, "by"), "at": col(r, "at"), "status": col(r, "status"),
        "decided_by": col(r, "decided_by"), "decided_at": col(r, "decided_at"), "note": col(r, "note"),
        "payload": serde_json::from_str::<Value>(&col(r, "payload")).unwrap_or(Value::Null),
    })
}

async fn list_changes(State(s): St, p: P, Path(org): Path<String>) -> Response {
    let principal = who(p);
    let s = s.as_ref();
    if let Err(r) = crate::org_policy::allowed(s, &principal, &org).await {
        return r;
    }
    let mut rows = query_platform_table(s, CHANGES, Some(format!("org = {}", sql_quote(&org))), 10_000).await.unwrap_or_default();
    rows.sort_by(|a, b| (col(b, "status") == "pending").cmp(&(col(a, "status") == "pending")).then(col(b, "at").cmp(&col(a, "at"))));
    rows.truncate(200);
    let me = principal.id().to_string();
    let can_approve = may_approve(s, &principal, &org).await.is_ok();
    let list: Vec<Value> = rows.iter().map(|r| {
        let mut c = change_json(r);
        c["can_decide"] = json!(can_approve && col(r, "status") == "pending" && col(r, "by") != me);
        c
    }).collect();
    Json(json!({ "org": org, "four_eyes": four_eyes(s, &org).await, "changes": list })).into_response()
}

async fn pending(s: &dyn StorageBackend, principal: &Principal, org: &str, id: &str) -> Result<Value, Response> {
    crate::org_policy::allowed(s, principal, org).await?;
    let row = query_platform_table(s, CHANGES, Some(format!("id = {} AND org = {}", sql_quote(id), sql_quote(org))), 1).await
        .ok().and_then(|r| r.into_iter().next())
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("{org} has no change {id}")))?;
    if col(&row, "status") != "pending" {
        return Err(err(StatusCode::CONFLICT, format!("change {id} was already {}", col(&row, "status"))));
    }
    if col(&row, "by") == principal.id() {
        return Err(err(StatusCode::FORBIDDEN, "a change is approved by someone other than its author"));
    }
    may_approve(s, principal, org).await?;
    Ok(row)
}

async fn decide(s: &dyn StorageBackend, row: &Value, status: &str, by: &str, note: &str) -> Result<(), String> {
    let db = db(s).await?;
    let mut r = row.clone();
    r["status"] = json!(status);
    r["decided_by"] = json!(by);
    r["decided_at"] = json!(chrono::Utc::now().to_rfc3339());
    r["note"] = json!(note);
    db.upsert(CHANGES, &["id"], &[r]).await.map(|_| ())
}

async fn approve(State(s): St, p: P, Path((org, id)): Path<(String, String)>, body: Option<Json<Value>>) -> Response {
    let principal = who(p);
    let s = s.as_ref();
    let row = match pending(s, &principal, &org, &id).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    let note: String = body.and_then(|Json(b)| b.get("note").and_then(|v| v.as_str()).map(str::to_string)).unwrap_or_default().chars().take(500).collect();
    let author = col(&row, "by");
    let payload: Value = serde_json::from_str(&col(&row, "payload")).unwrap_or(Value::Null);
    let subject = col(&row, "subject");
    // Applied as it would have been, checked against the files as they are now.
    let applied = match (col(&row, "target").as_str(), col(&row, "op").as_str()) {
        ("file", "save") => crate::org_policy::save_file(s, &principal, &org, &subject, payload["content"].as_str().unwrap_or_default(), payload["shadow"].as_bool().unwrap_or(false), Some(&author)).await,
        ("file", "delete") => crate::org_policy::remove_file(s, &principal, &org, &subject, Some(&author)).await,
        ("file", "enforce") => crate::org_policy::enforce(s, &principal, &org, &payload, Some(&author)).await,
        ("agent", "save") => crate::agent_space::security::save_policy(s, &principal, &subject, payload["policy"].as_str().unwrap_or_default(), Some(&author)).await,
        ("agent", "delete") => crate::agent_space::security::drop_policy(s, &principal, &subject, Some(&author)).await,
        ("agent-record", "update") => apply_agent_update(s, &principal, &payload, &author).await,
        ("settings", "retention") => crate::audit_retention::set(s, &principal, &org, &payload, Some(&author)).await,
        ("settings", "four_eyes") => set_four_eyes(s, &principal, &org, payload["four_eyes"].as_bool().unwrap_or(true), Some(&author)).await,
        (t, o) => Err(err(StatusCode::UNPROCESSABLE_ENTITY, format!("unknown change {t}/{o}"))),
    };
    let summary = col(&row, "summary");
    match applied {
        Ok(resp) if resp.status().is_success() => {
            if let Err(e) = decide(s, &row, "approved", principal.id(), &note).await {
                return internal(e);
            }
            flowr_policy::audit::event(&principal, "policy:approve", "PolicyChange", &id, None, Some(&org), None,
                Some(&format!("{summary} · by {author}{}", if note.is_empty() { String::new() } else { format!(" · {note}") })));
            Json(json!({ "approved": true, "change": id, "applied": true })).into_response()
        }
        // It no longer works with the files as they are now: it stays pending
        // and the reasons are returned (fix forward, or reject it).
        Ok(resp) | Err(resp) => resp,
    }
}

async fn reject(State(s): St, p: P, Path((org, id)): Path<(String, String)>, body: Option<Json<Value>>) -> Response {
    let principal = who(p);
    let s = s.as_ref();
    let row = match pending(s, &principal, &org, &id).await {
        Ok(r) => r,
        Err(r) => {
            // The author may withdraw their own change.
            let own = query_platform_table(s, CHANGES, Some(format!("id = {} AND org = {} AND status = 'pending'", sql_quote(&id), sql_quote(&org))), 1).await
                .ok().and_then(|r| r.into_iter().next()).filter(|r| col(r, "by") == principal.id());
            match own {
                Some(r) => r,
                None => return r,
            }
        }
    };
    let note: String = body.and_then(|Json(b)| b.get("note").and_then(|v| v.as_str()).map(str::to_string)).unwrap_or_default().chars().take(500).collect();
    let status = if col(&row, "by") == principal.id() { "withdrawn" } else { "rejected" };
    if let Err(e) = decide(s, &row, status, principal.id(), &note).await {
        return internal(e);
    }
    flowr_policy::audit::event(&principal, "policy:reject", "PolicyChange", &id, None, Some(&org), None,
        Some(&format!("{} {} · by {}", status, col(&row, "summary"), col(&row, "by"))));
    Json(json!({ status: true, "change": id })).into_response()
}
