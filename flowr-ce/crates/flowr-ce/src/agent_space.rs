//! Agents as repos, FlowR side: an agent's memory space and its branches, tags,
//! layers, fork and promote (`.ai/A3T_BRANCHING_API.md` §6, spec
//! `.ai/A3T_AGENTS_AS_REPOS_SPEC.md` §4).
//!
//! A memory space is a **namespace in the org's ADB** (the org's shard and sequencer;
//! a space is not a shard). Until per-org ADB resources exist, every org uses
//! [`ORG_ADB`]. Which namespace is an agent's is recorded per agent in the platform
//! table `agent_memory` (`agent_id`, `org_id`, `namespace`, `updated_by`,
//! `updated_at`): `agent_configs` has a fixed column set, and existing agents keep the
//! namespace their template names (`call:a3t:agent` `memory_namespace`) until they get
//! a row. New agents get `<project>--<agent>` ([`default_namespace`]).
//!
//! REST, under the auth middleware (a signed-in principal):
//!
//! | Route | ADB action |
//! |---|---|
//! | `GET` / `PUT /agents/{id}/memory` | none (the `agent_memory` row) |
//! | `GET` / `POST /agents/{id}/branches`, `DELETE /agents/{id}/branches/{name}` | `branch_list` / `branch_create` / `branch_drop` |
//! | `POST /agents/{id}/branches/{name}/promote` | `branch_promote`, then the new agent's rows |
//! | `POST /agents/{id}/fork` | `space_clone`, then the new agent's rows |
//! | `GET` / `POST /agents/{id}/tags`, `DELETE /agents/{id}/tags/{name}` | `tag_list` / `tag_create` / `tag_drop` |
//! | `GET` / `PUT /agents/{id}/layers` | `layers_get` / `layers_set` |
//!
//! Reads need read access to the agent's project (its `namespace` column), writes
//! need write access (`access::rest_write`). ADB refusals come back as `400
//! {error}` with ADB's sentence.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use serde_json::{json, Map, Value};

use flowr_policy::namespace::ItemKind;
use flowr_policy::Principal;
use flowr_storage::StorageBackend;

use crate::handlers::call_adb::{flight_action, resolve_adb};
use crate::handlers::call_lancedb::{query_platform_table, sql_quote, DEFAULT_PLATFORM_DB_PATH, PLATFORM_DB_RESOURCE};
use crate::platform_db::open_platform_db;

/// The org's ADB (the agents' memory). One for every org until per-org resources exist.
pub const ORG_ADB: &str = "a3t/agent-memory";
pub const TABLE: &str = "agent_memory";
const COLUMNS: [&str; 5] = ["agent_id", "org_id", "namespace", "updated_by", "updated_at"];

/// The ADB resource of an org's memory. `A3T_ADB_RESOURCE` overrides the default,
/// as for runtime pods.
pub fn org_adb(_org_id: &str) -> String {
    std::env::var("A3T_ADB_RESOURCE").ok().filter(|r| !r.is_empty()).unwrap_or_else(|| ORG_ADB.to_string())
}

/// `<project>--<agent>`, reduced to what a namespace may hold (no `|` or `~`).
pub fn default_namespace(project: &str, agent_id: &str) -> String {
    let clean = |s: &str| s.chars().map(|c| if c == '|' || c == '~' || c.is_whitespace() { '-' } else { c }).collect::<String>();
    let project = if project.is_empty() { "default" } else { project };
    format!("{}--{}", clean(project), clean(agent_id))
}

fn valid_namespace(ns: &str) -> bool {
    !ns.is_empty() && ns.len() <= 200 && !ns.contains('|') && !ns.contains('~')
}

/// The agent behind a chat session id (`<agent-id>:session-…`), if any.
pub fn agent_of_session(session_id: &str) -> Option<&str> {
    let (agent, _) = session_id.split_once(':')?;
    (!agent.is_empty()).then_some(agent)
}

/// An agent's memory space: the org's ADB and the agent's namespace in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Space {
    pub agent_id: String,
    pub org_id: String,
    pub resource: String,
    pub namespace: String,
}

/// The space recorded for `agent_id`; `None` when it has none (the table may not exist yet).
pub async fn space_of(storage: &dyn StorageBackend, agent_id: &str) -> Option<Space> {
    let rows = query_platform_table(storage, TABLE, Some(format!("agent_id = {}", sql_quote(agent_id))), 1).await.ok()?;
    let row = rows.into_iter().next()?;
    let s = |k: &str| row.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let namespace = s("namespace");
    if namespace.is_empty() {
        return None;
    }
    let org_id = s("org_id");
    Some(Space { agent_id: agent_id.to_string(), resource: org_adb(&org_id), org_id, namespace })
}

/// Record `namespace` as `agent_id`'s space (insert or replace).
pub async fn set_space(storage: &dyn StorageBackend, agent_id: &str, org_id: &str, namespace: &str, by: &str) -> Result<(), String> {
    if !valid_namespace(namespace) {
        return Err(format!("namespace {namespace:?}: 1–200 characters, no '|' or '~'"));
    }
    let db = open_platform_db(storage, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let schema: Map<String, Value> = COLUMNS.iter().map(|c| (c.to_string(), json!("string"))).collect();
    db.create_table(TABLE, Some(&schema), "create_if_not_exists").await?;
    let row = json!({
        "agent_id": agent_id, "org_id": org_id, "namespace": namespace,
        "updated_by": by, "updated_at": chrono::Utc::now().to_rfc3339(),
    });
    db.upsert(TABLE, &["agent_id"], &[row]).await.map(|_| ())
}

// ── REST ────────────────────────────────────────────────────────────────────

type Reply = Result<Response, Response>;

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

/// An ADB refusal as the REST answer (BR-17): `{error, code}` with the status
/// that fits ADB's stable code (DB-18), so the console can offer the fix.
fn adb_refusal(e: &str) -> Response {
    if e.starts_with("ADB request to") {
        return err(StatusCode::BAD_GATEWAY, e);
    }
    let (code, msg) = crate::handlers::call_adb::adb_code(e);
    let msg = msg.trim_start_matches("ADB: ").to_string();
    let status = match code.as_deref() {
        Some("not_found" | "branch_not_found" | "tag_not_found" | "merge_not_found") => StatusCode::NOT_FOUND,
        Some("branch_exists" | "tag_exists" | "key_exists" | "key_on_branch" | "target_not_empty"
            | "branch_has_children" | "tag_pinned" | "merged_record"
            // Merges (DB-10, DB-19, DB-11; ADB .ai/MERGE.md): the answer lists what to resolve.
            | "merge_conflicts" | "merge_stale" | "revert_conflicts") => StatusCode::CONFLICT,
        Some("forbidden" | "sealed_key") => StatusCode::FORBIDDEN,
        Some("unauthorized" | "internal" | "wrong_engine" | "durable_required") => StatusCode::BAD_GATEWAY,
        Some("timeout") => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::BAD_REQUEST,
    };
    match code {
        Some(c) => (status, Json(json!({ "error": msg, "code": c }))).into_response(),
        None => err(status, msg),
    }
}

fn who(p: Option<Extension<Principal>>) -> Principal {
    p.map(|Extension(p)| p).unwrap_or_else(|| Principal::service("anonymous"))
}

async fn agent_row(storage: &dyn StorageBackend, id: &str) -> Result<Value, Response> {
    let rows = query_platform_table(storage, "agent_configs", Some(format!("id = {}", sql_quote(id))), 1)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    rows.into_iter().next().ok_or_else(|| err(StatusCode::NOT_FOUND, format!("no agent {id:?}")))
}

fn col(row: &Value, k: &str) -> String {
    row.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

fn project_of(row: &Value) -> String {
    let p = col(row, "namespace");
    if p.is_empty() { "default".into() } else { p }
}

/// The agent (checked for access) and its space. `write`: write access to its project.
async fn agent_space(storage: &dyn StorageBackend, principal: &Principal, id: &str, write: bool) -> Result<(Value, Space), Response> {
    let row = agent_row(storage, id).await?;
    let project = project_of(&row);
    if write {
        crate::access::rest_write(principal, &project).await.map_err(IntoResponse::into_response)?;
    } else if !crate::access::can_read_item(principal, ItemKind::Agent, &project, &col(&row, "name"), "").await {
        return Err(err(StatusCode::NOT_FOUND, format!("no agent {id:?}")));
    }
    let space = space_of(storage, id).await.ok_or_else(|| {
        (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("agent {id:?} has no memory space of its own yet: set one with PUT /agents/{id}/memory"),
                "default_namespace": default_namespace(&project, id),
            })),
        )
            .into_response()
    })?;
    Ok((row, space))
}

async fn adb(storage: &dyn StorageBackend, space: &Space, action: &str, mut body: Value) -> Result<Value, Response> {
    let ep = resolve_adb(storage, &space.resource).await.map_err(|e| err(StatusCode::BAD_GATEWAY, e))?;
    if let Some(m) = body.as_object_mut() {
        m.entry("namespace").or_insert_with(|| json!(space.namespace));
    }
    flight_action(&ep, action, &body).await.map_err(|e| adb_refusal(&e))
}

fn ok(v: Value) -> Reply {
    Ok(Json(v).into_response())
}

fn both(r: Reply) -> Response {
    r.unwrap_or_else(|e| e)
}

pub fn router(storage: Arc<dyn StorageBackend>) -> Router {
    Router::new()
        .route("/agents/:agent_id/memory", get(get_memory).put(put_memory))
        .route("/agents/:agent_id/branches", get(list_branches).post(create_branch))
        .route("/agents/:agent_id/branches/:name", delete(drop_branch))
        .route("/agents/:agent_id/branches/:name/promote", post(promote_branch))
        // Merges (UX "Two kinds of merge"; ADB .ai/MERGE.md, DB-10/DB-19). Bodies
        // pass through to ADB, so its field names stay the contract.
        .route("/agents/:agent_id/branches/:name/conflicts", get(branch_conflicts))
        .route("/agents/:agent_id/branches/:name/merge", post(merge_branch))
        .route("/agents/:agent_id/branches/:name/rebase", post(rebase_branch))
        .route("/agents/:agent_id/merges/:merge_id/revert", post(revert_merge))
        .route("/agents/:agent_id/fork", post(fork_agent))
        .route("/agents/:agent_id/tags", get(list_tags).post(create_tag))
        .route("/agents/:agent_id/tags/:name", delete(drop_tag))
        .route("/agents/:agent_id/layers", get(get_layers).put(put_layers))
        .with_state(storage)
}

type St = State<Arc<dyn StorageBackend>>;
type P = Option<Extension<Principal>>;

async fn get_memory(State(s): St, p: P, Path(id): Path<String>) -> Response {
    both(async {
        let principal = who(p);
        let row = agent_row(s.as_ref(), &id).await?;
        let project = project_of(&row);
        if !crate::access::can_read_item(&principal, ItemKind::Agent, &project, &col(&row, "name"), "").await {
            return Err(err(StatusCode::NOT_FOUND, format!("no agent {id:?}")));
        }
        ok(match space_of(s.as_ref(), &id).await {
            Some(sp) => json!({ "agent_id": id, "resource": sp.resource, "namespace": sp.namespace, "own": true }),
            None => json!({ "agent_id": id, "resource": org_adb(&col(&row, "org_id")), "namespace": null, "own": false,
                            "default_namespace": default_namespace(&project, &id) }),
        })
    }.await)
}

/// `{namespace?}`: the agent's space; default `<project>--<agent>`.
async fn put_memory(State(s): St, p: P, Path(id): Path<String>, body: Option<Json<Value>>) -> Response {
    both(async {
        let principal = who(p);
        let row = agent_row(s.as_ref(), &id).await?;
        let project = project_of(&row);
        crate::access::rest_write(&principal, &project).await.map_err(IntoResponse::into_response)?;
        let ns = body.as_ref().and_then(|b| b.get("namespace")).and_then(|v| v.as_str()).map(str::to_string)
            .unwrap_or_else(|| default_namespace(&project, &id));
        set_space(s.as_ref(), &id, &col(&row, "org_id"), &ns, principal.id()).await.map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
        ok(json!({ "agent_id": id, "resource": org_adb(&col(&row, "org_id")), "namespace": ns, "own": true }))
    }.await)
}

async fn list_branches(State(s): St, p: P, Path(id): Path<String>) -> Response {
    both(async {
        let (_, sp) = agent_space(s.as_ref(), &who(p), &id, false).await?;
        ok(adb(s.as_ref(), &sp, "branch_list", json!({})).await?)
    }.await)
}

/// `{name, parent?}`.
async fn create_branch(State(s): St, p: P, Path(id): Path<String>, Json(b): Json<Value>) -> Response {
    both(async {
        let principal = who(p);
        let (_, sp) = agent_space(s.as_ref(), &principal, &id, true).await?;
        let name = b.get("name").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "name is required"))?;
        let mut body = json!({ "name": name, "created_by": principal.id() });
        if let Some(parent) = b.get("parent").and_then(|v| v.as_str()) {
            body["parent"] = json!(parent);
        }
        let r = adb(s.as_ref(), &sp, "branch_create", body).await?;
        Ok((StatusCode::CREATED, Json(r)).into_response())
    }.await)
}

async fn drop_branch(State(s): St, p: P, Path((id, name)): Path<(String, String)>) -> Response {
    both(async {
        let (_, sp) = agent_space(s.as_ref(), &who(p), &id, true).await?;
        ok(adb(s.as_ref(), &sp, "branch_drop", json!({ "name": name })).await?)
    }.await)
}

/// The new agent of a promote or fork: a copy of `source`'s row with a new id and
/// name, in the same project, and its own space.
fn new_agent_row(source: &Value, b: &Value, by: &str) -> Result<(String, Value), Response> {
    let spec = b.get("new_agent").unwrap_or(b);
    let id = spec.get("id").and_then(|v| v.as_str()).map(str::to_string)
        .unwrap_or_else(|| format!("agt-{}", &uuid::Uuid::now_v7().simple().to_string()[..12]));
    if id.contains('|') || id.contains('~') || id.contains(':') || id.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, format!("agent id {id:?}: no '|', '~' or ':'")));
    }
    let name = spec.get("name").and_then(|v| v.as_str()).map(str::to_string)
        .unwrap_or_else(|| format!("{} (copy)", col(source, "name")));
    let mut row = source.clone();
    let m = row.as_object_mut().ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "agent row is not an object"))?;
    m.insert("id".into(), json!(id));
    m.insert("name".into(), json!(name));
    m.insert("created_by".into(), json!(by));
    m.insert("created_at".into(), json!(chrono::Utc::now().to_rfc3339()));
    Ok((id, row))
}

async fn write_new_agent(s: &dyn StorageBackend, row: &Value, id: &str, org: &str, ns: &str, by: &str) -> Result<(), Response> {
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    db.insert("agent_configs", std::slice::from_ref(row)).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("memory copied into {ns}, but the agent row failed: {e}")))?;
    set_space(s, id, org, ns, by).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("agent {id} created, but its memory space was not recorded: {e}")))
}

/// Shared by promote and fork: the new agent must not exist, its namespace is
/// `<project>--<new id>`.
async fn copy_into_new_agent(s: &dyn StorageBackend, principal: &Principal, id: &str, b: &Value, action: &str, extra: Value) -> Reply {
    let (row, sp) = agent_space(s, principal, id, true).await?;
    let (new_id, new_row) = new_agent_row(&row, b, principal.id())?;
    if agent_row(s, &new_id).await.is_ok() {
        return Err(err(StatusCode::CONFLICT, format!("agent {new_id:?} already exists")));
    }
    let into = default_namespace(&project_of(&row), &new_id);
    let mut body = extra;
    body["into"] = json!(into);
    let copied = adb(s, &sp, action, body).await?;
    write_new_agent(s, &new_row, &new_id, &sp.org_id, &into, principal.id()).await?;
    Ok((StatusCode::CREATED, Json(json!({ "agent": new_row, "memory": copied }))).into_response())
}

/// `{new_agent: {id?, name?}}`.
async fn promote_branch(State(s): St, p: P, Path((id, name)): Path<(String, String)>, body: Option<Json<Value>>) -> Response {
    let b = body.map(|Json(b)| b).unwrap_or_else(|| json!({}));
    both(copy_into_new_agent(s.as_ref(), &who(p), &id, &b, "branch_promote", json!({ "name": name })).await)
}

/// Three-way diff of a branch against its base and main now: `{base, main, clean,
/// conflicts}`. `?memories=PROCEDURAL,SEMANTIC` narrows it.
async fn branch_conflicts(
    State(s): St, p: P, Path((id, name)): Path<(String, String)>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    both(async {
        let (_, sp) = agent_space(s.as_ref(), &who(p), &id, false).await?;
        let mut body = json!({ "name": name });
        if let Some(m) = q.get("memories").filter(|m| !m.is_empty()) {
            body["memories"] = json!(m.split(',').map(|x| x.trim().to_uppercase()).collect::<Vec<_>>());
        }
        ok(adb(s.as_ref(), &sp, "branch_conflicts", body).await?)
    }.await)
}

/// The caller's body for a merge or rebase, with the branch and who did it set by
/// the server (a body can't name someone else).
fn merge_body(b: Option<Json<Value>>, name: &str, by: &str) -> Value {
    let mut body = b.map(|Json(b)| b).filter(|b| b.is_object()).unwrap_or_else(|| json!({}));
    body["name"] = json!(name);
    body["merged_by"] = json!(by);
    body
}

/// Kind 2, procedures back into this agent: `{main_offset, resolutions, memories?}`
/// → `{merge_id, offset, changed, added, ended}`; 409 `merge_conflicts` /
/// `merge_stale` with what to resolve.
async fn merge_branch(State(s): St, p: P, Path((id, name)): Path<(String, String)>, body: Option<Json<Value>>) -> Response {
    both(async {
        let principal = who(p);
        let (_, sp) = agent_space(s.as_ref(), &principal, &id, true).await?;
        ok(adb(s.as_ref(), &sp, "branch_merge", merge_body(body, &name, principal.id())).await?)
    }.await)
}

/// Kind 1's first step: move the branch onto main now with resolutions, so a
/// promote carries prod's newer memory. `{main_offset, resolutions}`.
async fn rebase_branch(State(s): St, p: P, Path((id, name)): Path<(String, String)>, body: Option<Json<Value>>) -> Response {
    both(async {
        let principal = who(p);
        let (_, sp) = agent_space(s.as_ref(), &principal, &id, true).await?;
        ok(adb(s.as_ref(), &sp, "branch_rebase", merge_body(body, &name, principal.id())).await?)
    }.await)
}

/// Undo a kind 2 merge: the previous versions come back as new versions; records
/// changed since come back as 409 `revert_conflicts`.
async fn revert_merge(State(s): St, p: P, Path((id, merge_id)): Path<(String, String)>) -> Response {
    both(async {
        let principal = who(p);
        let (_, sp) = agent_space(s.as_ref(), &principal, &id, true).await?;
        ok(adb(s.as_ref(), &sp, "merge_revert", json!({ "merge_id": merge_id, "reverted_by": principal.id() })).await?)
    }.await)
}

/// `{new_agent: {id?, name?}}`.
async fn fork_agent(State(s): St, p: P, Path(id): Path<String>, body: Option<Json<Value>>) -> Response {
    let b = body.map(|Json(b)| b).unwrap_or_else(|| json!({}));
    both(copy_into_new_agent(s.as_ref(), &who(p), &id, &b, "space_clone", json!({})).await)
}

async fn list_tags(State(s): St, p: P, Path(id): Path<String>) -> Response {
    both(async {
        let (_, sp) = agent_space(s.as_ref(), &who(p), &id, false).await?;
        ok(adb(s.as_ref(), &sp, "tag_list", json!({})).await?)
    }.await)
}

/// `{name}`.
async fn create_tag(State(s): St, p: P, Path(id): Path<String>, Json(b): Json<Value>) -> Response {
    both(async {
        let principal = who(p);
        let (_, sp) = agent_space(s.as_ref(), &principal, &id, true).await?;
        let name = b.get("name").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "name is required"))?;
        let r = adb(s.as_ref(), &sp, "tag_create", json!({ "name": name, "created_by": principal.id() })).await?;
        Ok((StatusCode::CREATED, Json(r)).into_response())
    }.await)
}

async fn drop_tag(State(s): St, p: P, Path((id, name)): Path<(String, String)>) -> Response {
    both(async {
        let (_, sp) = agent_space(s.as_ref(), &who(p), &id, true).await?;
        ok(adb(s.as_ref(), &sp, "tag_drop", json!({ "name": name })).await?)
    }.await)
}

async fn get_layers(State(s): St, p: P, Path(id): Path<String>) -> Response {
    both(async {
        let (_, sp) = agent_space(s.as_ref(), &who(p), &id, false).await?;
        ok(adb(s.as_ref(), &sp, "layers_get", json!({})).await?)
    }.await)
}

/// `{layers: [{agent | space, mode, tag?, memories, sealed?}]}`: a layer names another
/// agent (`agent`, resolved to its space; read access needed) or a namespace (`space`).
async fn put_layers(State(s): St, p: P, Path(id): Path<String>, Json(b): Json<Value>) -> Response {
    both(async {
        let principal = who(p);
        let (_, sp) = agent_space(s.as_ref(), &principal, &id, true).await?;
        let mut layers = Vec::new();
        for l in b.get("layers").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
            let mut l = l;
            if let Some(agent) = l.get("agent").and_then(|v| v.as_str()).map(str::to_string) {
                let (_, other) = agent_space(s.as_ref(), &principal, &agent, false).await?;
                if other.org_id != sp.org_id {
                    return Err(err(StatusCode::BAD_REQUEST, format!("agent {agent:?} is in another org: layers stay in one org's memory")));
                }
                let m = l.as_object_mut().ok_or_else(|| err(StatusCode::BAD_REQUEST, "a layer is an object"))?;
                m.remove("agent");
                m.insert("space".into(), json!(other.namespace));
            }
            layers.push(l);
        }
        ok(adb(s.as_ref(), &sp, "layers_set", json!({ "layers": layers })).await?)
    }.await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_namespaces() {
        assert_eq!(default_namespace("acme", "support-bot"), "acme--support-bot");
        assert_eq!(default_namespace("", "x"), "default--x");
        assert_eq!(default_namespace("a|b", "c~d"), "a-b--c-d");
    }

    #[test]
    fn sessions_name_their_agent() {
        assert_eq!(agent_of_session("agt-1:session-9"), Some("agt-1"));
        assert_eq!(agent_of_session("session-9"), None);
        assert_eq!(agent_of_session(":x"), None);
    }

    #[test]
    fn a_new_agent_copies_the_source_row() {
        let src = json!({"id": "a1", "name": "Bot", "template_id": "t", "namespace": "acme", "org_id": "o"});
        let (id, row) = new_agent_row(&src, &json!({"new_agent": {"id": "a2", "name": "Bot v2"}}), "u1").unwrap();
        assert_eq!(id, "a2");
        assert_eq!(row["template_id"], "t");
        assert_eq!(row["name"], "Bot v2");
        assert_eq!(row["created_by"], "u1");
        assert!(new_agent_row(&src, &json!({"id": "a|b"}), "u").is_err());
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    #[test]
    fn a_merge_body_names_the_branch_and_the_caller_whatever_it_says() {
        let b = Some(Json(json!({ "main_offset": 41, "resolutions": [], "name": "other", "merged_by": "someone-else" })));
        let body = merge_body(b, "try-new-recall", "usr-ann");
        assert_eq!(body["name"], "try-new-recall");
        assert_eq!(body["merged_by"], "usr-ann");
        assert_eq!(body["main_offset"], 41, "the rest passes through unchanged");
        assert_eq!(merge_body(None, "b", "u"), json!({ "name": "b", "merged_by": "u" }));
    }
}
