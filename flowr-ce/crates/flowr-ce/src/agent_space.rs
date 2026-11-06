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
//! | `GET` / `POST /agents/{id}/versions`, `GET /agents/{id}/versions/{n}` | `tag_create`, the `agent_versions` row (AC-5) |
//! | `GET /agents/{id}/deployments`, `PUT /agents/{id}/deployments/{env}` | none (the `agent_deployments` row) |
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

pub mod environments;
mod merge_requests;

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
    // Always the agent's own space: a request body can never aim an action at
    // another agent's memory (merge bodies pass through from the caller).
    if let Some(m) = body.as_object_mut() {
        m.insert("namespace".into(), json!(space.namespace));
    }
    flight_action(&ep, action, &body).await.map_err(|e| adb_refusal(&e))
}

/// AC-14: the policy's decision on a lifecycle action on this agent, before it happens. A
/// refusal is logged and answers `403 {error, code: "policy_denied"}`; there's nobody to
/// ask mid-request, so an `Ask` is a refusal. CE allows everything.
async fn allowed(principal: &Principal, action: &str, agent_id: &str) -> Result<(), Response> {
    let action = flowr_policy::Action::new(action).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let req = flowr_policy::ExecutionRequest {
        principal: principal.clone(),
        action: action.clone(),
        resource: flowr_policy::Resource::new("Agent", agent_id),
        context: Default::default(),
        attributes: Default::default(),
        params: Value::Null,
    };
    let reason = match flowr_policy::policy_hook().check(&req).await.non_interactive() {
        flowr_policy::PolicyDecision::Allow => return Ok(()),
        flowr_policy::PolicyDecision::Deny { reason, policy_ids } => {
            tracing::warn!(agent = agent_id, principal = %principal.id(), action = ?action, %reason, ?policy_ids, "lifecycle action denied by policy");
            reason
        }
        flowr_policy::PolicyDecision::Ask { .. } => {
            tracing::warn!(agent = agent_id, principal = %principal.id(), action = ?action, "lifecycle action needs an approval, refused");
            "this needs an approval".to_string()
        }
    };
    Err((StatusCode::FORBIDDEN, Json(json!({ "error": format!("Policy refused this: {reason}"), "code": "policy_denied" }))).into_response())
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
        .route("/agents/:agent_id/versions", get(list_versions).post(create_version))
        .route("/agents/:agent_id/versions/:version", get(get_version))
        .route("/agents/:agent_id/deployments", get(list_deployments))
        .route("/agents/:agent_id/runs", get(list_runs))
        .route("/agents/:agent_id/deployments/:environment", axum::routing::put(pin_deployment))
        .merge(merge_requests::routes())
        .merge(environments::routes())
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
        allowed(&principal, flowr_policy::actions::MEMORY_BRANCH, &id).await?;
        let name = b.get("name").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "name is required"))?;
        let mut body = json!({ "name": name, "created_by": principal.id() });
        if let Some(parent) = b.get("parent").and_then(|v| v.as_str()) {
            body["parent"] = json!(parent);
        }
        let r = adb(s.as_ref(), &sp, "branch_create", body).await?;
        merge_requests::snapshot_branch_flows(s.as_ref(), &id, name).await?;
        // D4: with a non-prod pod, branch memory lives there, off prod's compute.
        let r = match environments::non_prod_flight() {
            Some(url) => adb(s.as_ref(), &sp, "branch_place", json!({ "name": name, "url": url })).await?,
            None => r,
        };
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
    let answer = copy_new_agent(s, principal, id, b, action, extra, Value::Null).await?;
    Ok((StatusCode::CREATED, Json(answer)).into_response())
}

/// As [`copy_into_new_agent`], with the flows of the new agent's first version.
async fn copy_new_agent(s: &dyn StorageBackend, principal: &Principal, id: &str, b: &Value, action: &str, extra: Value, flows: Value) -> Result<Value, Response> {
    let (row, sp) = agent_space(s, principal, id, true).await?;
    allowed(principal, if action == "space_clone" { flowr_policy::actions::MEMORY_FORK } else { flowr_policy::actions::MEMORY_PROMOTE }, id).await?;
    let (new_id, new_row) = new_agent_row(&row, b, principal.id())?;
    if agent_row(s, &new_id).await.is_ok() {
        return Err(err(StatusCode::CONFLICT, format!("agent {new_id:?} already exists")));
    }
    let into = default_namespace(&project_of(&row), &new_id);
    let branch = extra.get("name").cloned();
    let mut body = extra;
    body["into"] = json!(into);
    let copied = adb(s, &sp, action, body).await?;
    write_new_agent(s, &new_row, &new_id, &sp.org_id, &into, principal.id()).await?;
    // AC-5: the new agent's first version says where it came from. The agent exists by
    // now, so a failure here is reported, not fatal.
    let lineage = match branch {
        Some(b) => json!({ "promoted_from": { "agent": id, "branch": b, "memory": copied.get("from") } }),
        None => json!({ "forked_from": { "agent": id, "memory": copied.get("from") } }),
    };
    let new_space = Space { agent_id: new_id.clone(), resource: sp.resource.clone(), org_id: sp.org_id.clone(), namespace: into.clone() };
    let mut answer = json!({ "agent": new_row, "memory": copied });
    match new_version(s, &new_row, &new_space, &flows, "", lineage, principal.id()).await {
        Ok(v) => answer["version"] = v,
        Err(_) => answer["version_error"] = json!(format!("agent {new_id} was created without a first version: create one with POST /agents/{new_id}/versions")),
    }
    Ok(answer)
}

/// `{new_agent: {id?, name?}, allow_stale?}`.
async fn promote_branch(State(s): St, p: P, Path((id, name)): Path<(String, String)>, body: Option<Json<Value>>) -> Response {
    let b = body.map(|Json(b)| b).unwrap_or_else(|| json!({}));
    let mut extra = json!({ "name": name });
    // Past ADB's gate (main changed since the branch point; rebase is the normal path).
    if b.get("allow_stale").and_then(|v| v.as_bool()) == Some(true) {
        extra["allow_stale"] = json!(true);
    }
    both(copy_into_new_agent(s.as_ref(), &who(p), &id, &b, "branch_promote", extra).await)
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
/// the server (a body can't name someone else); `adb()` sets the namespace.
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
        allowed(&principal, flowr_policy::actions::MEMORY_MERGE, &id).await?;
        ok(adb(s.as_ref(), &sp, "branch_merge", merge_body(body, &name, principal.id())).await?)
    }.await)
}

/// Kind 1's first step: move the branch onto main now with resolutions, so a
/// promote carries prod's newer memory. `{main_offset, resolutions}`.
async fn rebase_branch(State(s): St, p: P, Path((id, name)): Path<(String, String)>, body: Option<Json<Value>>) -> Response {
    both(async {
        let principal = who(p);
        let (_, sp) = agent_space(s.as_ref(), &principal, &id, true).await?;
        allowed(&principal, flowr_policy::actions::MEMORY_MERGE, &id).await?;
        ok(adb(s.as_ref(), &sp, "branch_rebase", merge_body(body, &name, principal.id())).await?)
    }.await)
}

/// Undo a kind 2 merge: the previous versions come back as new versions; records
/// changed since come back as 409 `revert_conflicts`.
/// `{force_skip?}`: revert what didn't change since and report the rest.
async fn revert_merge(State(s): St, p: P, Path((id, merge_id)): Path<(String, String)>, body: Option<Json<Value>>) -> Response {
    both(async {
        let principal = who(p);
        let (_, sp) = agent_space(s.as_ref(), &principal, &id, true).await?;
        allowed(&principal, flowr_policy::actions::MEMORY_MERGE, &id).await?;
        let mut req = json!({ "merge_id": merge_id, "reverted_by": principal.id() });
        if let Some(Json(b)) = body {
            if let Some(f) = b.get("force_skip").and_then(|v| v.as_bool()) {
                req["force_skip"] = json!(f);
            }
        }
        ok(adb(s.as_ref(), &sp, "merge_revert", req).await?)
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

/// The agent whose memory space is `namespace`, if any.
async fn agent_of_space(storage: &dyn StorageBackend, namespace: &str) -> Result<Option<String>, Response> {
    let rows = query_platform_table(storage, TABLE, Some(format!("namespace = {}", sql_quote(namespace))), 1)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(rows.first().and_then(|r| r.get("agent_id")).and_then(|v| v.as_str()).map(str::to_string))
}

/// The layers to store: what the caller asked for, minus any history layer, plus
/// the history layers the space has now. A promoted agent's history layer (D1) is
/// made by promote; a caller can neither add one (it would read another agent's
/// conversations) nor lose it by sending a list without it, since `layers_set`
/// replaces the whole list.
fn with_history_layers(requested: Vec<Value>, current: &Value) -> Vec<Value> {
    let is_history = |l: &Value| l.get("mode").and_then(|m| m.as_str()) == Some("history");
    let now = current.get("layers").map(|l| l.get("layers").unwrap_or(l)).and_then(|l| l.as_array());
    let mut out: Vec<Value> = requested.into_iter().filter(|l| !is_history(l)).collect();
    out.extend(now.into_iter().flatten().filter(|l| is_history(l)).cloned());
    out
}

/// `{layers: [{agent | space, mode, tag?, memories, sealed?}]}`: a layer names another
/// agent, by id (`agent`) or by its namespace (`space`, as `GET` answers). Either way
/// the caller needs read access to that agent, and it must be in this agent's org: a
/// namespace no agent owns is refused, so a layer can't reach another org's memory.
async fn put_layers(State(s): St, p: P, Path(id): Path<String>, Json(b): Json<Value>) -> Response {
    both(async {
        let principal = who(p);
        let (_, sp) = agent_space(s.as_ref(), &principal, &id, true).await?;
        allowed(&principal, flowr_policy::actions::MEMORY_LAYERS, &id).await?;
        let requested = b.get("layers").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let current = adb(s.as_ref(), &sp, "layers_get", json!({})).await?;
        let mut layers = Vec::new();
        for l in with_history_layers(requested, &json!({})) {
            let mut l = l;
            let m = l.as_object_mut().ok_or_else(|| err(StatusCode::BAD_REQUEST, "a layer is an object"))?;
            let agent = match (m.remove("agent").and_then(|v| v.as_str().map(str::to_string)), m.get("space").and_then(|v| v.as_str())) {
                (Some(agent), _) => agent,
                (None, Some(space)) => agent_of_space(s.as_ref(), space).await?
                    .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("no agent has the memory space {space:?}: a layer names an agent in this org")))?,
                (None, None) => return Err(err(StatusCode::BAD_REQUEST, "a layer names an agent or its space")),
            };
            let (_, other) = agent_space(s.as_ref(), &principal, &agent, false).await?;
            if other.org_id != sp.org_id {
                return Err(err(StatusCode::BAD_REQUEST, format!("agent {agent:?} is in another org: layers stay in one org's memory")));
            }
            m.insert("space".into(), json!(other.namespace));
            layers.push(l);
        }
        let layers = with_history_layers(layers, &current);
        ok(adb(s.as_ref(), &sp, "layers_set", json!({ "layers": layers })).await?)
    }.await)
}

// ── Agent versions and deployments (AC-5) ───────────────────────────────────

/// `agent_versions`: what one version of an agent is, never changed once written.
/// `agent_deployments`: which version each environment runs. The JSON columns are text,
/// as in every platform table.
pub const VERSIONS: &str = "agent_versions";
pub const DEPLOYMENTS: &str = "agent_deployments";
const VERSION_COLUMNS: [&str; 10] = ["agent_id", "version", "template", "flows", "memory", "layers", "lineage", "note", "created_by", "created_at"];
const DEPLOYMENT_COLUMNS: [&str; 6] = ["agent_id", "environment", "version", "previous", "deployed_by", "deployed_at"];

async fn version_rows(s: &dyn StorageBackend, id: &str) -> Result<Vec<Value>, Response> {
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let schema: Map<String, Value> = VERSION_COLUMNS.iter().map(|c| (c.to_string(), json!("string"))).collect();
    db.create_table(VERSIONS, Some(&schema), "create_if_not_exists").await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let mut rows = query_platform_table(s, VERSIONS, Some(format!("agent_id = {}", sql_quote(id))), 100_000)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    rows.sort_by_key(version_number);
    Ok(rows)
}

fn version_number(row: &Value) -> u64 {
    col(row, "version").parse().unwrap_or(0)
}

/// A stored row as the API shows it: the JSON columns parsed.
fn version_json(row: &Value) -> Value {
    let mut v = row.clone();
    if let Some(m) = v.as_object_mut() {
        for k in ["template", "flows", "memory", "layers", "lineage"] {
            let parsed = m.get(k).and_then(|x| x.as_str()).and_then(|x| serde_json::from_str::<Value>(x).ok()).unwrap_or(Value::Null);
            m.insert(k.to_string(), parsed);
        }
    }
    v
}

/// Write the next version of the agent: its template, `flows` (each checked against the
/// registry), its memory at this moment as an ADB tag, and its layers. The tag keeps
/// that memory past the 90 days of history.
async fn new_version(s: &dyn StorageBackend, row: &Value, sp: &Space, flows: &Value, note: &str, lineage: Value, by: &str) -> Result<Value, Response> {
    let id = &sp.agent_id;
    let flows = flows.as_array().cloned().unwrap_or_default();
    for f in &flows {
        let (ns, name, version) = (col(f, "namespace"), col(f, "name"), col(f, "version"));
        if s.workflows().get_version(&ns, &name, &version).await.is_err() {
            return Err(err(StatusCode::BAD_REQUEST, format!("flows: {ns}/{name} has no version {version:?}")));
        }
    }
    let number = version_rows(s, id).await?.iter().map(version_number).max().unwrap_or(0) + 1;
    let tag = format!("agent-v{number}");
    let tagged = adb(s, sp, "tag_create", json!({ "name": tag, "created_by": by })).await?;
    let layers = adb(s, sp, "layers_get", json!({})).await?.get("layers").cloned().unwrap_or(json!([]));
    let memory = json!({ "namespace": sp.namespace, "tag": tag, "point": { "offset": tagged["point_offset"], "ts": tagged["point_ts"] } });
    let template = json!({ "id": col(row, "template_id"), "version": col(row, "template_version") });
    let stored = json!({
        "agent_id": id,
        "version": number.to_string(),
        "template": template.to_string(),
        "flows": Value::Array(flows).to_string(),
        "memory": memory.to_string(),
        "layers": layers.to_string(),
        "lineage": lineage.to_string(),
        "note": note,
        "created_by": by,
        "created_at": chrono::Utc::now().to_rfc3339(),
    });
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    db.insert(VERSIONS, std::slice::from_ref(&stored)).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("memory tagged {tag}, but the version row failed: {e}")))?;
    Ok(version_json(&stored))
}

async fn list_versions(State(s): St, p: P, Path(id): Path<String>) -> Response {
    both(async {
        agent_space(s.as_ref(), &who(p), &id, false).await?;
        let rows = version_rows(s.as_ref(), &id).await?;
        ok(json!({ "versions": rows.iter().map(version_json).collect::<Vec<_>>() }))
    }.await)
}

/// `{flows?: [{namespace, name, version}], note?}`. There is no update: a change is the
/// next version.
async fn create_version(State(s): St, p: P, Path(id): Path<String>, body: Option<Json<Value>>) -> Response {
    both(async {
        let principal = who(p);
        let b = body.map(|Json(b)| b).unwrap_or_else(|| json!({}));
        let (row, sp) = agent_space(s.as_ref(), &principal, &id, true).await?;
        let v = new_version(s.as_ref(), &row, &sp, b.get("flows").unwrap_or(&Value::Null), &col(&b, "note"), Value::Null, principal.id()).await?;
        Ok((StatusCode::CREATED, Json(v)).into_response())
    }.await)
}

async fn get_version(State(s): St, p: P, Path((id, version)): Path<(String, String)>) -> Response {
    both(async {
        agent_space(s.as_ref(), &who(p), &id, false).await?;
        let rows = version_rows(s.as_ref(), &id).await?;
        match rows.iter().find(|r| col(r, "version") == version) {
            Some(r) => ok(version_json(r)),
            None => Err(err(StatusCode::NOT_FOUND, format!("agent {id:?} has no version {version:?}"))),
        }
    }.await)
}

/// AC-20: `agent_runs`, one row per agent turn: which version in which environment, and
/// the memory it read. Best effort: a failed write is logged and never fails the turn.
pub const RUNS: &str = "agent_runs";
const RUN_COLUMNS: [&str; 8] = ["run_id", "agent_id", "agent_version", "environment", "memory_namespace", "memory_branch", "memory_point", "at"];

/// The environment this process serves (`A3T_ENVIRONMENT`, default `prod`).
pub fn environment() -> String {
    std::env::var("A3T_ENVIRONMENT").ok().filter(|e| !e.is_empty()).unwrap_or_else(|| "prod".to_string())
}

pub async fn record_run(s: &dyn StorageBackend, run_id: &str, sp: &Space, branch: Option<&str>, point: Option<Value>) {
    let environment = environment();
    let version = async {
        // The pinned version here, else the latest.
        let pinned = deployment_rows(s, &sp.agent_id).await.ok()?.iter().find(|r| col(r, "environment") == environment).map(|r| col(r, "version"));
        match pinned {
            Some(v) => Some(v),
            None => version_rows(s, &sp.agent_id).await.ok()?.last().map(|r| col(r, "version")),
        }
    }
    .await
    .unwrap_or_default();
    let row = json!({
        "run_id": run_id, "agent_id": sp.agent_id, "agent_version": version, "environment": environment,
        "memory_namespace": sp.namespace, "memory_branch": branch.unwrap_or(BRANCH_MAIN),
        "memory_point": point.map(|p| p.to_string()).unwrap_or_default(), "at": chrono::Utc::now().to_rfc3339(),
    });
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let schema: Map<String, Value> = RUN_COLUMNS.iter().map(|c| (c.to_string(), json!("string"))).collect();
    let written = async {
        db.create_table(RUNS, Some(&schema), "create_if_not_exists").await?;
        db.insert(RUNS, std::slice::from_ref(&row)).await
    };
    if let Err(e) = written.await {
        tracing::warn!(error = %e, run = run_id, "recording the agent run failed");
    }
}

const BRANCH_MAIN: &str = "main";

/// An agent's recorded turns. Order isn't guaranteed: sort by `at`.
async fn list_runs(State(s): St, p: P, Path(id): Path<String>) -> Response {
    both(async {
        agent_space(s.as_ref(), &who(p), &id, false).await?;
        let db = open_platform_db(s.as_ref(), Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
        let schema: Map<String, Value> = RUN_COLUMNS.iter().map(|c| (c.to_string(), json!("string"))).collect();
        db.create_table(RUNS, Some(&schema), "create_if_not_exists").await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let rows = query_platform_table(s.as_ref(), RUNS, Some(format!("agent_id = {}", sql_quote(&id))), 1000).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        ok(json!({ "runs": rows }))
    }.await)
}

async fn deployment_rows(s: &dyn StorageBackend, id: &str) -> Result<Vec<Value>, Response> {
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let schema: Map<String, Value> = DEPLOYMENT_COLUMNS.iter().map(|c| (c.to_string(), json!("string"))).collect();
    db.create_table(DEPLOYMENTS, Some(&schema), "create_if_not_exists").await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    query_platform_table(s, DEPLOYMENTS, Some(format!("agent_id = {}", sql_quote(id))), 1000).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))
}

async fn list_deployments(State(s): St, p: P, Path(id): Path<String>) -> Response {
    both(async {
        agent_space(s.as_ref(), &who(p), &id, false).await?;
        ok(json!({ "deployments": deployment_rows(s.as_ref(), &id).await? }))
    }.await)
}

/// `{version}`: pin the environment to that version. Rolling back is pinning the previous
/// one. This records the pin; moving the traffic is the controller's (AC-11a).
async fn pin_deployment(State(s): St, p: P, Path((id, environment)): Path<(String, String)>, Json(b): Json<Value>) -> Response {
    both(async {
        let principal = who(p);
        let (agent, _) = agent_space(s.as_ref(), &principal, &id, true).await?;
        let env = environments::find(s.as_ref(), &project_of(&agent), &environment).await?;
        if environments::is_production(&env) {
            allowed(&principal, flowr_policy::actions::DEPLOY_PROD, &id).await?;
        }
        // AC-10: an environment with an approval rule takes its pins through a merge request.
        if environments::needs_approval(&env) {
            return Err((StatusCode::CONFLICT, Json(json!({
                "error": format!("{environment} needs approval: open a merge request with kind \"deploy\", environment and version"),
                "code": "approval_required",
            }))).into_response());
        }
        let version = b.get("version").map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string())).unwrap_or_default();
        ok(write_pin(s.as_ref(), &id, &environment, &version, principal.id()).await?)
    }.await)
}

/// Pin `environment` to `version`, remembering the one before.
async fn write_pin(s: &dyn StorageBackend, id: &str, environment: &str, version: &str, by: &str) -> Result<Value, Response> {
    if !version_rows(s, id).await?.iter().any(|r| col(r, "version") == version) {
        return Err(err(StatusCode::NOT_FOUND, format!("agent {id:?} has no version {version:?}")));
    }
    let previous = deployment_rows(s, id).await?.iter().find(|r| col(r, "environment") == environment).map(|r| col(r, "version")).unwrap_or_default();
    let row = json!({
        "agent_id": id, "environment": environment, "version": version, "previous": previous,
        "deployed_by": by, "deployed_at": chrono::Utc::now().to_rfc3339(),
    });
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    db.upsert(DEPLOYMENTS, &["agent_id", "environment"], std::slice::from_ref(&row)).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(row)
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
    fn history_layers_can_neither_be_added_nor_lost() {
        let old = json!({ "space": "acme--bot", "mode": "history", "tag": "promoted-acme--bot2", "memories": ["EPISODIC"] });
        let pin = json!({ "space": "acme--compliance", "mode": "pin", "tag": "v1", "memories": ["SEMANTIC"] });
        let forged = json!({ "space": "other--secret", "mode": "history", "tag": "t", "memories": ["EPISODIC"] });
        // A list without it keeps it; a forged one is dropped. Both answer shapes.
        for current in [json!({ "layers": [old.clone(), pin.clone()] }), json!({ "layers": { "layers": [old.clone()] } })] {
            assert_eq!(with_history_layers(vec![pin.clone(), forged.clone()], &current), vec![pin.clone(), old.clone()]);
        }
        // No layers yet.
        assert_eq!(with_history_layers(vec![pin.clone()], &json!({ "layers": null })), vec![pin.clone()]);
        assert!(with_history_layers(vec![forged], &json!({})).is_empty());
    }

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
