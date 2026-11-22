//! Access reviews (GOV-11): a snapshot of who holds what in an org — its
//! members' org roles, people's roles on its projects, its agents' live keys —
//! that its admins and security admins go through, keeping or revoking each.
//! A revoke applies at once (the membership, the project role or the key goes),
//! every decision is in the decision log (`access_review:keep|revoke`), and a
//! review closes when every item is decided.
//!
//! - `GET|POST /security/orgs/{org}/reviews` `{name}`
//! - `GET  /security/orgs/{org}/reviews/{id}`
//! - `POST /security/orgs/{org}/reviews/{id}/items/{item}` `{decision: keep|revoke, note?}`
//! - `POST /security/orgs/{org}/reviews/{id}/close`

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

pub const REVIEWS: &str = "access_reviews";
const REVIEW_COLUMNS: [&str; 7] = ["id", "org", "name", "status", "created_by", "created_at", "closed_at"];
pub const ITEMS: &str = "access_review_items";
const ITEM_COLUMNS: [&str; 10] = ["id", "review", "kind", "subject", "detail", "target", "decision", "decided_by", "decided_at", "note"];

type St = State<Arc<dyn StorageBackend>>;
type P = Option<Extension<Principal>>;
type Reply = Result<Response, Response>;

fn col(row: &Value, k: &str) -> String {
    row.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}
fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}
fn internal(e: impl ToString) -> Response {
    err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
fn who(p: P) -> Principal {
    p.map(|Extension(p)| p).unwrap_or_else(|| Principal::service("anonymous"))
}
fn both(r: Reply) -> Response {
    r.unwrap_or_else(|e| e)
}

pub fn routes() -> Router<Arc<dyn StorageBackend>> {
    Router::new()
        .route("/security/orgs/:org/reviews", get(list).post(create))
        .route("/security/orgs/:org/reviews/:id", get(one))
        .route("/security/orgs/:org/reviews/:id/items/:item", post(decide))
        .route("/security/orgs/:org/reviews/:id/close", post(close))
}

async fn db(s: &dyn StorageBackend) -> Result<Arc<dyn PlatformDb>, String> {
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    for (t, cols) in [(REVIEWS, &REVIEW_COLUMNS[..]), (ITEMS, &ITEM_COLUMNS[..])] {
        let schema: Map<String, Value> = cols.iter().map(|c| (c.to_string(), json!("string"))).collect();
        db.create_table(t, Some(&schema), "create_if_not_exists").await?;
    }
    Ok(db)
}

/// Who holds what in `org` now: `(kind, subject, detail, target)`.
pub async fn snapshot(s: &dyn StorageBackend, org: &str, org_id: &str) -> Vec<(String, String, String, Value)> {
    let mut out = Vec::new();
    for m in query_platform_table(s, "org_members", Some(format!("org_id = {}", sql_quote(org_id))), 100_000).await.unwrap_or_default() {
        let user = col(&m, "user_id");
        let label = [col(&m, "email"), col(&m, "name")].into_iter().find(|x| !x.is_empty()).unwrap_or_else(|| user.clone());
        out.push(("org_role".into(), label, format!("org {}", col(&m, "role")), json!({ "org_id": org_id, "user_id": user })));
    }
    let projects = query_platform_table(s, "projects", Some(format!("owner_kind = 'org' AND owner = {}", sql_quote(org))), 10_000).await.unwrap_or_default();
    for p in &projects {
        let ns = col(p, "namespace");
        let members: Vec<Value> = serde_json::from_str(&col(p, "members")).unwrap_or_default();
        for m in members {
            let user = m.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let role = m.get("role").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            if user.is_empty() { continue; }
            out.push(("project_role".into(), user.clone(), format!("{role} on {ns}"), json!({ "namespace": ns, "user_id": user })));
        }
    }
    let agents = query_platform_table(s, "agent_configs", Some(format!("org_id = {}", sql_quote(org_id))), 100_000).await.unwrap_or_default();
    let keys = query_platform_table(s, crate::agent_keys::TABLE, Some(format!("org_id = {}", sql_quote(org_id))), 100_000).await.unwrap_or_default();
    let now = chrono::Utc::now();
    for k in keys {
        if !col(&k, "revoked_at").is_empty() { continue; }
        if chrono::DateTime::parse_from_rfc3339(&col(&k, "expires_at")).is_ok_and(|t| t <= now) { continue; }
        let agent = col(&k, "agent_id");
        let name = agents.iter().find(|a| col(a, "id") == agent).map(|a| col(a, "name")).unwrap_or_else(|| agent.clone());
        out.push(("agent_key".into(), format!("{} ({}…)", col(&k, "name"), col(&k, "prefix")),
            format!("key of agent {name}, made by {}, last used {}", col(&k, "created_by"), if col(&k, "last_used_at").is_empty() { "never".into() } else { col(&k, "last_used_at") }),
            json!({ "agent_id": agent, "key_id": col(&k, "id") })));
    }
    out
}

async fn org_id(s: &dyn StorageBackend, org: &str) -> Result<String, Response> {
    crate::org_policy::org_id(s, org).await.ok_or_else(|| err(StatusCode::NOT_FOUND, format!("no org {org:?}")))
}

async fn list(State(s): St, p: P, Path(org): Path<String>) -> Response {
    both(async {
        let s = s.as_ref();
        crate::org_policy::allowed(s, &who(p), &org).await?;
        let _ = db(s).await.map_err(internal)?;
        let mut reviews = query_platform_table(s, REVIEWS, Some(format!("org = {}", sql_quote(&org))), 1000).await.unwrap_or_default();
        reviews.sort_by(|a, b| col(b, "created_at").cmp(&col(a, "created_at")));
        let mut out = Vec::new();
        for r in reviews {
            let items = query_platform_table(s, ITEMS, Some(format!("review = {}", sql_quote(&col(&r, "id")))), 100_000).await.unwrap_or_default();
            out.push(json!({
                "id": col(&r, "id"), "name": col(&r, "name"), "status": col(&r, "status"), "created_by": col(&r, "created_by"),
                "created_at": col(&r, "created_at"), "closed_at": col(&r, "closed_at"), "items": items.len(),
                "pending": items.iter().filter(|i| col(i, "decision") == "pending").count(),
                "revoked": items.iter().filter(|i| col(i, "decision") == "revoke").count(),
            }));
        }
        Ok(Json(json!({ "org": org, "reviews": out })).into_response())
    }.await)
}

async fn create(State(s): St, p: P, Path(org): Path<String>, body: Option<Json<Value>>) -> Response {
    both(async {
        let principal = who(p);
        let s = s.as_ref();
        crate::org_policy::allowed(s, &principal, &org).await?;
        let oid = org_id(s, &org).await?;
        let db = db(s).await.map_err(internal)?;
        let id = uuid::Uuid::now_v7().to_string();
        let name: String = body.as_ref().map(|Json(b)| col(b, "name")).filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| format!("Review of {}", chrono::Utc::now().format("%Y-%m-%d"))).chars().take(120).collect();
        let items: Vec<Value> = snapshot(s, &org, &oid).await.into_iter().map(|(kind, subject, detail, target)| json!({
            "id": uuid::Uuid::now_v7().to_string(), "review": id, "kind": kind, "subject": subject, "detail": detail,
            "target": target.to_string(), "decision": "pending", "decided_by": "", "decided_at": "", "note": "",
        })).collect();
        if !items.is_empty() {
            db.insert(ITEMS, &items).await.map_err(internal)?;
        }
        db.insert(REVIEWS, &[json!({ "id": id, "org": org, "name": name, "status": "open", "created_by": principal.id(),
            "created_at": chrono::Utc::now().to_rfc3339(), "closed_at": "" })]).await.map_err(internal)?;
        flowr_policy::audit::event(&principal, "access_review:start", "AccessReview", &id, None, Some(&oid), None, Some(&format!("{name}: {} items", items.len())));
        Ok((StatusCode::CREATED, Json(json!({ "id": id, "items": items.len() }))).into_response())
    }.await)
}

async fn review_of(s: &dyn StorageBackend, org: &str, id: &str) -> Result<Value, Response> {
    query_platform_table(s, REVIEWS, Some(format!("id = {} AND org = {}", sql_quote(id), sql_quote(org))), 1).await.ok()
        .and_then(|r| r.into_iter().next()).ok_or_else(|| err(StatusCode::NOT_FOUND, format!("{org} has no review {id}")))
}

async fn one(State(s): St, p: P, Path((org, id)): Path<(String, String)>) -> Response {
    both(async {
        let s = s.as_ref();
        crate::org_policy::allowed(s, &who(p), &org).await?;
        let r = review_of(s, &org, &id).await?;
        let mut items = query_platform_table(s, ITEMS, Some(format!("review = {}", sql_quote(&id))), 100_000).await.unwrap_or_default();
        items.sort_by(|a, b| (col(a, "kind"), col(a, "subject")).cmp(&(col(b, "kind"), col(b, "subject"))));
        let n = |d: &str| items.iter().filter(|i| col(i, "decision") == d).count();
        let items_json: Vec<Value> = items.iter().map(|i| json!({
            "id": col(i, "id"), "kind": col(i, "kind"), "subject": col(i, "subject"), "detail": col(i, "detail"),
            "target": serde_json::from_str::<Value>(&col(i, "target")).unwrap_or(Value::Null), "decision": col(i, "decision"),
            "decided_by": col(i, "decided_by"), "decided_at": col(i, "decided_at"), "note": col(i, "note"),
        })).collect();
        Ok(Json(json!({ "review": {
            "id": id, "name": col(&r, "name"), "status": col(&r, "status"), "created_by": col(&r, "created_by"), "created_at": col(&r, "created_at"),
            "closed_at": col(&r, "closed_at"), "items": items.len(), "pending": n("pending"), "revoked": n("revoke"),
        }, "items": items_json })).into_response())
    }.await)
}

/// Take away what an item holds.
async fn revoke(s: &dyn StorageBackend, db: &dyn PlatformDb, kind: &str, target: &Value) -> Result<(), String> {
    let t = |k: &str| target.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    match kind {
        "org_role" => db.delete("org_members", &format!("org_id = {} AND user_id = {}", sql_quote(&t("org_id")), sql_quote(&t("user_id")))).await,
        "project_role" => {
            let ns = t("namespace");
            let row = query_platform_table(s, "projects", Some(format!("namespace = {}", sql_quote(&ns))), 1).await?.into_iter().next()
                .ok_or_else(|| format!("no project {ns}"))?;
            let members: Vec<Value> = serde_json::from_str(&col(&row, "members")).unwrap_or_default();
            let kept: Vec<Value> = members.into_iter().filter(|m| m.get("id").and_then(|v| v.as_str()) != Some(t("user_id").as_str())).collect();
            let mut v = Map::new();
            v.insert("members".into(), json!(serde_json::to_string(&kept).unwrap_or_default()));
            db.update("projects", &format!("namespace = {}", sql_quote(&ns)), &v).await
        }
        "agent_key" => crate::agent_keys::revoke(s, &t("agent_id"), &t("key_id")).await.map(|_| ()),
        other => Err(format!("unknown item kind {other}")),
    }?;
    crate::access::invalidate();
    Ok(())
}

async fn decide(State(s): St, p: P, Path((org, id, item)): Path<(String, String, String)>, Json(b): Json<Value>) -> Response {
    both(async {
        let principal = who(p);
        let s = s.as_ref();
        crate::org_policy::allowed(s, &principal, &org).await?;
        let r = review_of(s, &org, &id).await?;
        if col(&r, "status") != "open" {
            return Err(err(StatusCode::CONFLICT, "the review is closed"));
        }
        let decision = col(&b, "decision");
        if decision != "keep" && decision != "revoke" {
            return Err(err(StatusCode::BAD_REQUEST, "decision: keep or revoke"));
        }
        let row = query_platform_table(s, ITEMS, Some(format!("id = {} AND review = {}", sql_quote(&item), sql_quote(&id))), 1).await.ok()
            .and_then(|r| r.into_iter().next()).ok_or_else(|| err(StatusCode::NOT_FOUND, "no such item"))?;
        if col(&row, "decision") != "pending" {
            return Err(err(StatusCode::CONFLICT, format!("already decided: {}", col(&row, "decision"))));
        }
        let db = db(s).await.map_err(internal)?;
        let target: Value = serde_json::from_str(&col(&row, "target")).unwrap_or(Value::Null);
        if decision == "revoke" {
            revoke(s, db.as_ref(), &col(&row, "kind"), &target).await.map_err(|e| err(StatusCode::BAD_GATEWAY, format!("not revoked: {e}")))?;
        }
        let mut v = Map::new();
        v.insert("decision".into(), json!(decision));
        v.insert("decided_by".into(), json!(principal.id()));
        v.insert("decided_at".into(), json!(chrono::Utc::now().to_rfc3339()));
        v.insert("note".into(), json!(col(&b, "note").chars().take(300).collect::<String>()));
        db.update(ITEMS, &format!("id = {}", sql_quote(&item)), &v).await.map_err(internal)?;
        let oid = crate::org_policy::org_id(s, &org).await;
        flowr_policy::audit::event(&principal, &format!("access_review:{decision}"), "AccessReviewItem", &item, None, oid.as_deref(), None,
            Some(&format!("{} · {} · {}", col(&row, "kind"), col(&row, "subject"), col(&row, "detail"))));
        Ok(Json(json!({ "item": item, "decision": decision })).into_response())
    }.await)
}

async fn close(State(s): St, p: P, Path((org, id)): Path<(String, String)>) -> Response {
    both(async {
        let principal = who(p);
        let s = s.as_ref();
        crate::org_policy::allowed(s, &principal, &org).await?;
        let r = review_of(s, &org, &id).await?;
        if col(&r, "status") != "open" {
            return Err(err(StatusCode::CONFLICT, "already closed"));
        }
        let items = query_platform_table(s, ITEMS, Some(format!("review = {}", sql_quote(&id))), 100_000).await.unwrap_or_default();
        let pending = items.iter().filter(|i| col(i, "decision") == "pending").count();
        if pending > 0 {
            return Err(err(StatusCode::CONFLICT, format!("{pending} item(s) still to decide")));
        }
        let db = db(s).await.map_err(internal)?;
        let mut v = Map::new();
        v.insert("status".into(), json!("closed"));
        v.insert("closed_at".into(), json!(chrono::Utc::now().to_rfc3339()));
        db.update(REVIEWS, &format!("id = {}", sql_quote(&id)), &v).await.map_err(internal)?;
        let revoked = items.iter().filter(|i| col(i, "decision") == "revoke").count();
        let oid = crate::org_policy::org_id(s, &org).await;
        flowr_policy::audit::event(&principal, "access_review:close", "AccessReview", &id, None, oid.as_deref(), None,
            Some(&format!("{} items, {revoked} revoked", items.len())));
        Ok(Json(json!({ "closed": true, "items": items.len(), "revoked": revoked })).into_response())
    }.await)
}
