//! How long an org's decisions are kept (AUD-6), and its legal hold.
//!
//! Each org sets `retention_days` (0: forever, the default) and `legal_hold`
//! (Security › the org › Policy files › Changes). Once a day the console pod
//! forgets, in `_audit.<org id>`, the decisions older than that — unless the org
//! is on legal hold, which stops every purge until it is lifted. ADB's FORGET
//! keeps history (FORGET semantics, 13 Nov), so a purge is itself auditable; each
//! one is logged as `audit:purge`. Shortening the retention or lifting a hold
//! needs the org's second approver when it has one (GOV-3).
//!
//! - `GET  /security/orgs/{org}/retention` · `PUT {retention_days, legal_hold}`

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Extension, Json, Router,
};
use serde_json::{json, Map, Value};

use flowr_policy::Principal;
use flowr_storage::StorageBackend;

use crate::handlers::call_lancedb::{query_platform_table, sql_quote, DEFAULT_PLATFORM_DB_PATH, PLATFORM_DB_RESOURCE};
use crate::platform_db::open_platform_db;

pub const TABLE: &str = "org_audit_retention";
const COLUMNS: [&str; 5] = ["org", "retention_days", "legal_hold", "updated_by", "updated_at"];
/// The longest a retention may be: ten years.
const MAX_DAYS: u64 = 3650;

type St = State<Arc<dyn StorageBackend>>;
type P = Option<Extension<Principal>>;

fn col(row: &Value, k: &str) -> String {
    row.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}
fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

pub fn routes() -> Router<Arc<dyn StorageBackend>> {
    Router::new().route("/security/orgs/:org/retention", get(get_retention).put(put_retention))
}

/// `(retention_days, legal_hold)` of an org (by slug); `(0, false)` when unset.
pub async fn of(s: &dyn StorageBackend, org: &str) -> (u64, bool) {
    let row = query_platform_table(s, TABLE, Some(format!("org = {}", sql_quote(org))), 1).await.ok().and_then(|r| r.into_iter().next());
    match row {
        Some(r) => (col(&r, "retention_days").parse().unwrap_or(0), col(&r, "legal_hold") == "true"),
        None => (0, false),
    }
}

async fn get_retention(State(s): St, p: P, Path(org): Path<String>) -> Response {
    let principal = p.map(|Extension(p)| p).unwrap_or_else(|| Principal::service("anonymous"));
    if let Err(r) = crate::org_policy::allowed(s.as_ref(), &principal, &org).await {
        return r;
    }
    let (days, hold) = of(s.as_ref(), &org).await;
    Json(json!({ "org": org, "retention_days": days, "legal_hold": hold })).into_response()
}

async fn put_retention(State(s): St, p: P, Path(org): Path<String>, Json(b): Json<Value>) -> Response {
    let principal = p.map(|Extension(p)| p).unwrap_or_else(|| Principal::service("anonymous"));
    match set(s.as_ref(), &principal, &org, &b, None).await {
        Ok(r) | Err(r) => r,
    }
}

/// Change an org's retention or hold. Shortening it, or lifting a hold, is a
/// change its second approver approves when it has one.
pub(crate) async fn set(s: &dyn StorageBackend, principal: &Principal, org: &str, b: &Value, approved: Option<&str>) -> Result<Response, Response> {
    crate::org_policy::allowed(s, principal, org).await?;
    let (days_now, hold_now) = of(s, org).await;
    let days = match b.get("retention_days") {
        None => days_now,
        Some(v) => v.as_u64().filter(|d| *d <= MAX_DAYS).ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("retention_days: 0 (forever) to {MAX_DAYS}")))?,
    };
    if days != 0 && days < 30 {
        return Err(err(StatusCode::BAD_REQUEST, "retention_days: keep decisions at least 30 days (0 keeps them forever)"));
    }
    let hold = b.get("legal_hold").and_then(|v| v.as_bool()).unwrap_or(hold_now);
    let shortens = (days != 0 && (days_now == 0 || days < days_now)) || (hold_now && !hold);
    if approved.is_none() && shortens && crate::policy_changes::four_eyes(s, org).await {
        return crate::policy_changes::propose(s, principal, org, "settings", "retention", "", json!({ "retention_days": days, "legal_hold": hold }),
            &format!("decision log: keep {} days, legal hold {}", if days == 0 { "forever".to_string() } else { days.to_string() }, if hold { "on" } else { "off" })).await;
    }
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let schema: Map<String, Value> = COLUMNS.iter().map(|c| (c.to_string(), json!("string"))).collect();
    db.create_table(TABLE, Some(&schema), "create_if_not_exists").await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let by = approved.map(str::to_string).unwrap_or_else(|| principal.id().to_string());
    db.upsert(TABLE, &["org"], &[json!({
        "org": org, "retention_days": days.to_string(), "legal_hold": hold.to_string(), "updated_by": by, "updated_at": chrono::Utc::now().to_rfc3339(),
    })]).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    flowr_policy::audit::event(principal, "audit:retention", "Org", org, None, Some(org), None,
        Some(&format!("keep {} · legal hold {}", if days == 0 { "forever".to_string() } else { format!("{days} days") }, if hold { "on" } else { "off" })));
    Ok(Json(json!({ "org": org, "retention_days": days, "legal_hold": hold })).into_response())
}

/// Once a day on the console pod: forget each org's decisions past its
/// retention, unless it is on legal hold.
pub fn spawn(storage: Arc<dyn StorageBackend>) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(300)).await;
        loop {
            purge(storage.as_ref()).await;
            tokio::time::sleep(Duration::from_secs(24 * 3600)).await;
        }
    });
}

pub async fn purge(s: &dyn StorageBackend) {
    let rows = query_platform_table(s, TABLE, None, 10_000).await.unwrap_or_default();
    let who = Principal::service("audit-retention");
    for r in rows {
        let org = col(&r, "org");
        let days: u64 = col(&r, "retention_days").parse().unwrap_or(0);
        if days == 0 || col(&r, "legal_hold") == "true" {
            continue;
        }
        let Some(org_id) = crate::org_policy::org_id(s, &org).await else { continue };
        let before = (chrono::Utc::now() - chrono::Duration::days(days as i64)).timestamp_millis();
        let aql = format!("FORGET FROM EPISODIC WHERE kind = \"decision\" AND ts_ms < {before} NAMESPACE {}",
            crate::handlers::agent_memory::aql_str(&crate::decision_log::namespace(&org_id)));
        match crate::decision_log::run(&aql).await {
            Ok(n) => {
                tracing::info!(org, days, forgotten = n, "decision log: retention applied");
                flowr_policy::audit::event(&who, "audit:purge", "Org", &org, None, Some(&org_id), None,
                    Some(&format!("{n} decisions older than {days} days forgotten")));
            }
            Err(e) => tracing::warn!(org, error = %e, "decision log: retention not applied"),
        }
    }
}
