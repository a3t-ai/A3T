//! Merge requests (UX "Two kinds of merge"; AC-22, AC-23, AC-10, AC-11a):
//! one record per merge, of either kind, with its conflicts, resolutions, approvals,
//! evidence and how to undo it. Applying it is gated: nothing lands until every
//! conflict is resolved and the approvals and evidence are there.
//!
//! Kind 2 (procedures back into this agent) applies with ADB's `branch_merge` and is
//! undone with `merge_revert`. Kind 1 (behaviour, as a new agent) rebases, promotes,
//! resolves the flows into the new agent's first version and moves the triggers; it is
//! undone by moving them back.
//!
//! Flows on a branch (AC-22): `agent_branch_flows` holds, per flow, the version at the
//! branch point (`base_version`) and the branch's draft (`branch_version`). A draft is a
//! normal immutable registry version that isn't made active. Prod is the active version
//! now. A flow both sides changed differently is a conflict.
//!
//! Kind `deploy` pins a version to an environment whose rule asks for approval (AC-12):
//! no conflicts, the same gate, and rolling back pins the version before.
//!
//! How many approvals and how much evidence are needed is the target environment's rule
//! (`environments.rs`); a merge's target is the project's production environment.

use super::*;

use axum::routing::put;

pub const MERGES: &str = "agent_merges";
pub const BRANCH_FLOWS: &str = "agent_branch_flows";
const MERGE_COLUMNS: [&str; 18] = [
    "id", "agent_id", "branch", "kind", "environment", "version", "status", "resolutions", "flow_resolutions", "approvals", "evidence", "required_approvals",
    "required_evidence", "result", "created_by", "created_at", "updated_by", "updated_at",
];
const FLOW_COLUMNS: [&str; 6] = ["agent_id", "branch", "namespace", "name", "base_version", "branch_version"];
/// The columns holding JSON text.
const JSON_COLUMNS: [&str; 5] = ["resolutions", "flow_resolutions", "approvals", "evidence", "result"];

pub(super) fn routes() -> Router<Arc<dyn StorageBackend>> {
    Router::new()
        .route("/agents/:agent_id/branches/:name/flows", get(list_branch_flows).post(save_branch_flow))
        .route("/agents/:agent_id/merge-requests", get(list_requests).post(create_request))
        .route("/agents/:agent_id/merge-requests/:mr", get(get_request))
        .route("/agents/:agent_id/merge-requests/:mr/resolutions", put(set_resolutions))
        .route("/agents/:agent_id/merge-requests/:mr/approvals", post(approve))
        .route("/agents/:agent_id/merge-requests/:mr/evidence", post(add_evidence))
        .route("/agents/:agent_id/merge-requests/:mr/apply", post(apply))
        .route("/agents/:agent_id/merge-requests/:mr/rollback", post(rollback))
}

async fn table(s: &dyn StorageBackend, name: &str, columns: &[&str]) -> Result<Arc<dyn crate::platform_db::PlatformDb>, Response> {
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let schema: Map<String, Value> = columns.iter().map(|c| (c.to_string(), json!("string"))).collect();
    db.create_table(name, Some(&schema), "create_if_not_exists").await.map_err(internal)?;
    Ok(db)
}

fn internal(e: String) -> Response {
    err(StatusCode::INTERNAL_SERVER_ERROR, e)
}

// ── Flows on a branch (AC-22) ───────────────────────────────────────────────

/// What a flow's three versions say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowState {
    /// The branch didn't change it: prod's stays.
    Unchanged,
    /// Only the branch changed it.
    Clean,
    /// Both changed it, to the same version.
    Same,
    Conflict,
}

pub fn flow_state(base: &str, prod: &str, branch: &str) -> FlowState {
    if branch == base {
        FlowState::Unchanged
    } else if prod == branch {
        FlowState::Same
    } else if prod == base {
        FlowState::Clean
    } else {
        FlowState::Conflict
    }
}

/// The version a resolution picks, of the versions in `row`. `edit` names a new version
/// made from both, which the caller checks exists.
fn resolved_version(row: &Value, res: Option<&Value>) -> Result<Option<String>, String> {
    let (base, prod, branch) = (col(row, "base_version"), col(row, "prod_version"), col(row, "branch_version"));
    match flow_state(&base, &prod, &branch) {
        FlowState::Unchanged => Ok(Some(prod)),
        FlowState::Clean | FlowState::Same => Ok(Some(branch)),
        FlowState::Conflict => match res.map(|r| col(r, "take")).as_deref() {
            Some("branch") => Ok(Some(branch)),
            Some("prod") => Ok(Some(prod)),
            Some("edit") => {
                let v = res.map(|r| col(r, "version")).unwrap_or_default();
                if v.is_empty() {
                    return Err(format!("{}/{}: an edit needs a version", col(row, "namespace"), col(row, "name")));
                }
                Ok(Some(v))
            }
            Some(other) => Err(format!("{}/{}: take is branch, prod or edit, not {other:?}", col(row, "namespace"), col(row, "name"))),
            None => Ok(None),
        },
    }
}

async fn branch_flow_rows(s: &dyn StorageBackend, agent: &str, branch: &str) -> Result<Vec<Value>, Response> {
    table(s, BRANCH_FLOWS, &FLOW_COLUMNS).await?;
    query_platform_table(s, BRANCH_FLOWS, Some(format!("agent_id = {} AND branch = {}", sql_quote(agent), sql_quote(branch))), 10_000).await.map_err(internal)
}

/// The branch's flows with prod's active version and their state.
async fn flow_diff(s: &dyn StorageBackend, agent: &str, branch: &str) -> Result<Vec<Value>, Response> {
    let mut out = Vec::new();
    for mut row in branch_flow_rows(s, agent, branch).await? {
        let prod = s.workflows().get(&col(&row, "namespace"), &col(&row, "name")).await.map(|r| r.version().to_string()).unwrap_or_default();
        let state = match flow_state(&col(&row, "base_version"), &prod, &col(&row, "branch_version")) {
            FlowState::Unchanged => "unchanged",
            FlowState::Clean => "clean",
            FlowState::Same => "same",
            FlowState::Conflict => "conflict",
        };
        row["prod_version"] = json!(prod);
        row["state"] = json!(state);
        out.push(row);
    }
    Ok(out)
}

/// At the branch point: the flows of the agent's latest version are the branch's base, so
/// a fix released to prod later shows as a change on main.
pub(super) async fn snapshot_branch_flows(s: &dyn StorageBackend, agent: &str, branch: &str) -> Result<(), Response> {
    let Some(latest) = version_rows(s, agent).await?.last().map(version_json) else { return Ok(()) };
    let flows = latest.get("flows").and_then(|f| f.as_array()).cloned().unwrap_or_default();
    if flows.is_empty() {
        return Ok(());
    }
    let db = table(s, BRANCH_FLOWS, &FLOW_COLUMNS).await?;
    let rows: Vec<Value> = flows
        .iter()
        .map(|f| json!({ "agent_id": agent, "branch": branch, "namespace": col(f, "namespace"), "name": col(f, "name"), "base_version": col(f, "version"), "branch_version": col(f, "version") }))
        .collect();
    db.insert(BRANCH_FLOWS, &rows).await.map(|_| ()).map_err(internal)
}

async fn list_branch_flows(State(s): St, p: P, Path((id, name)): Path<(String, String)>) -> Response {
    both(async {
        agent_space(s.as_ref(), &who(p), &id, false).await?;
        ok(json!({ "flows": flow_diff(s.as_ref(), &id, &name).await? }))
    }.await)
}

/// `{yaml}`: save a draft of a flow on the branch. It's a new registry version, left
/// inactive, so prod keeps running what it runs. A flow the branch hasn't touched yet
/// takes prod's active version as its base.
async fn save_branch_flow(State(s): St, p: P, Path((id, name)): Path<(String, String)>, Json(b): Json<Value>) -> Response {
    both(async {
        let principal = who(p);
        agent_space(s.as_ref(), &principal, &id, true).await?;
        let yaml = b.get("yaml").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "yaml is required"))?;
        let mut record = flowr_types::WorkflowRecord::from_yaml(yaml).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
        crate::access::rest_write(&principal, record.namespace()).await.map_err(IntoResponse::into_response)?;
        flowr_mcp_core::flow_base::load_definition(s.as_ref(), &record).await.map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
        flowr_mcp_core::flow_base::check_version(s.as_ref(), &record).await.map_err(|e| (StatusCode::CONFLICT, Json(json!({ "error": e, "code": "version_exists" }))).into_response())?;
        let (ns, flow, version) = (record.namespace().to_string(), record.name().to_string(), record.version().to_string());
        let active = s.workflows().get(&ns, &flow).await.ok();
        record.metadata.owner = active.as_ref().and_then(|r| r.metadata.owner.clone()).or_else(|| Some(principal.id().to_string()));
        let active_version = active.as_ref().map(|r| r.version().to_string());
        s.workflows().register(record).await.map_err(|e| internal(e.to_string()))?;
        // Registering makes a version active; a draft mustn't be.
        if let Some(v) = &active_version {
            s.workflows().set_active_version(&ns, &flow, v).await.map_err(|e| internal(e.to_string()))?;
        }
        let existing = branch_flow_rows(s.as_ref(), &id, &name).await?.into_iter().find(|r| col(r, "namespace") == ns && col(r, "name") == flow);
        let base = existing.map(|r| col(&r, "base_version")).or(active_version).unwrap_or_default();
        let row = json!({ "agent_id": id, "branch": name, "namespace": ns, "name": flow, "base_version": base, "branch_version": version });
        let db = table(s.as_ref(), BRANCH_FLOWS, &FLOW_COLUMNS).await?;
        db.upsert(BRANCH_FLOWS, &["agent_id", "branch", "namespace", "name"], std::slice::from_ref(&row)).await.map_err(internal)?;
        Ok((StatusCode::CREATED, Json(row)).into_response())
    }.await)
}

// ── The merge record (AC-23) ────────────────────────────────────────────────

/// The rule of the environment a request changes: `(environment, approvals, evidence)`.
async fn rule_for(s: &dyn StorageBackend, project: &str, environment: Option<&str>) -> Result<(String, String, String), Response> {
    let env = match environment {
        Some(name) => environments::find(s, project, name).await?,
        None => {
            let all = environments::of_project(s, project).await.map_err(internal)?;
            all.into_iter().find(environments::is_production).ok_or_else(|| err(StatusCode::CONFLICT, format!("project {project:?} has no production environment")))?
        }
    };
    Ok((col(&env, "name"), col(&env, "required_approvals"), col(&env, "required_evidence")))
}

fn request_json(row: &Value) -> Value {
    let mut v = row.clone();
    if let Some(m) = v.as_object_mut() {
        for k in JSON_COLUMNS {
            let parsed = m.get(k).and_then(|x| x.as_str()).and_then(|x| serde_json::from_str::<Value>(x).ok()).unwrap_or(Value::Null);
            m.insert(k.to_string(), parsed);
        }
    }
    v
}

fn request_row(v: &Value, by: &str) -> Value {
    let mut r = v.clone();
    if let Some(m) = r.as_object_mut() {
        for k in JSON_COLUMNS {
            let text = m.get(k).cloned().unwrap_or(Value::Null).to_string();
            m.insert(k.to_string(), json!(text));
        }
        for k in ["conflicts", "flows", "main", "base", "blocked_by", "manifest_diff"] {
            m.remove(k);
        }
        m.insert("updated_by".into(), json!(by));
        m.insert("updated_at".into(), json!(chrono::Utc::now().to_rfc3339()));
    }
    r
}

async fn save(s: &dyn StorageBackend, v: &Value, by: &str) -> Result<(), Response> {
    let db = table(s, MERGES, &MERGE_COLUMNS).await?;
    db.upsert(MERGES, &["id"], &[request_row(v, by)]).await.map(|_| ()).map_err(internal)
}

async fn load(s: &dyn StorageBackend, agent: &str, mr: &str) -> Result<Value, Response> {
    table(s, MERGES, &MERGE_COLUMNS).await?;
    let filter = format!("id = {} AND agent_id = {}", sql_quote(mr), sql_quote(agent));
    let rows = query_platform_table(s, MERGES, Some(filter), 1).await.map_err(internal)?;
    rows.first().map(request_json).ok_or_else(|| err(StatusCode::NOT_FOUND, format!("agent {agent:?} has no merge request {mr:?}")))
}

/// What the record can't hold still: the conflicts as they are now, and what's in the way.
async fn with_live_state(s: &dyn StorageBackend, sp: &Space, mut v: Value) -> Result<Value, Response> {
    if col(&v, "status") != "open" {
        return Ok(v);
    }
    let kind = col(&v, "kind");
    if kind == "deploy" {
        v["blocked_by"] = json!(blocked_by(&v));
        return Ok(v);
    }
    let mut body = json!({ "name": col(&v, "branch") });
    if kind == "2" {
        body["memories"] = json!(["PROCEDURAL"]);
    }
    let d = adb(s, sp, "branch_conflicts", body).await?;
    v["conflicts"] = d.get("conflicts").cloned().unwrap_or(json!([]));
    v["base"] = d.get("base").cloned().unwrap_or(Value::Null);
    v["main"] = d.get("main").cloned().unwrap_or(Value::Null);
    if kind == "1" {
        let flows = flow_diff(s, &sp.agent_id, &col(&v, "branch")).await?;
        // AC-16: what this merge adds to what the agent can reach, by the flows it would run.
        let as_of = |key: &str| -> Vec<Value> {
            flows.iter().map(|f| json!({ "namespace": col(f, "namespace"), "name": col(f, "name"), "version": col(f, key) })).collect()
        };
        let current = manifest::of_flows(s, &as_of("prod_version")).await;
        let proposed = manifest::of_flows(s, &as_of("branch_version")).await;
        v["manifest_diff"] = manifest::diff(&current, &proposed);
        v["flows"] = json!(flows);
    }
    v["blocked_by"] = json!(blocked_by(&v));
    Ok(v)
}

/// AC-10: why this merge can't apply yet. Empty: it can.
pub fn blocked_by(v: &Value) -> Vec<String> {
    let list = |k: &str| v.get(k).and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::new();
    let resolved = list("resolutions");
    for c in list("conflicts") {
        if !resolved.iter().any(|r| col(r, "lineage") == col(&c, "lineage")) {
            out.push(format!("memory conflict on {}: resolve it", col(&c, "key")));
        }
    }
    let flow_res = list("flow_resolutions");
    for f in list("flows") {
        let chosen = flow_res.iter().find(|r| col(r, "namespace") == col(&f, "namespace") && col(r, "name") == col(&f, "name"));
        match resolved_version(&f, chosen) {
            Ok(Some(_)) => {}
            Ok(None) => out.push(format!("flow conflict on {}/{}: resolve it", col(&f, "namespace"), col(&f, "name"))),
            Err(e) => out.push(e),
        }
    }
    let need = |k: &str| v.get(k).and_then(|x| x.as_str()).and_then(|x| x.parse::<usize>().ok()).unwrap_or(0);
    let (have_a, need_a) = (list("approvals").len(), need("required_approvals"));
    if have_a < need_a {
        out.push(format!("approvals: {have_a} of {need_a}"));
    }
    let (have_e, need_e) = (list("evidence").len(), need("required_evidence"));
    if have_e < need_e {
        out.push(format!("evidence: {have_e} of {need_e}"));
    }
    out
}

/// `{branch, kind: 1 | 2}`. Kind 1: behaviour, as a new agent. Kind 2: procedures, back
/// into this agent. `{kind: "deploy", environment, version}`: pin a version.
async fn create_request(State(s): St, p: P, Path(id): Path<String>, Json(b): Json<Value>) -> Response {
    both(async {
        let principal = who(p);
        let (agent, sp) = agent_space(s.as_ref(), &principal, &id, true).await?;
        let kind = b.get("kind").map(|k| k.as_str().map(String::from).unwrap_or_else(|| k.to_string())).unwrap_or_default();
        let branch = col(&b, "branch");
        let version = b.get("version").map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string())).unwrap_or_default();
        let target = match kind.as_str() {
            "1" | "2" => {
                if branch.is_empty() {
                    return Err(err(StatusCode::BAD_REQUEST, "branch is required"));
                }
                None
            }
            "deploy" => {
                if col(&b, "environment").is_empty() || version.is_empty() {
                    return Err(err(StatusCode::BAD_REQUEST, "a deploy needs environment and version"));
                }
                if !version_rows(s.as_ref(), &id).await?.iter().any(|r| col(r, "version") == version) {
                    return Err(err(StatusCode::NOT_FOUND, format!("agent {id:?} has no version {version:?}")));
                }
                Some(col(&b, "environment"))
            }
            _ => return Err(err(StatusCode::BAD_REQUEST, "kind is 1 (a new agent), 2 (procedures back into this agent) or deploy")),
        };
        let (environment, approvals, evidence) = rule_for(s.as_ref(), &project_of(&agent), target.as_deref()).await?;
        let now = chrono::Utc::now().to_rfc3339();
        let v = json!({
            "id": format!("mr-{}", &uuid::Uuid::now_v7().simple().to_string()[..16]),
            "agent_id": id, "branch": branch, "kind": kind, "environment": environment, "version": version, "status": "open",
            "resolutions": [], "flow_resolutions": [], "approvals": [], "evidence": [],
            "required_approvals": approvals, "required_evidence": evidence,
            "result": null, "created_by": principal.id(), "created_at": now,
        });
        // Reading the conflicts first refuses a branch that doesn't exist.
        let live = with_live_state(s.as_ref(), &sp, v).await?;
        save(s.as_ref(), &live, principal.id()).await?;
        Ok((StatusCode::CREATED, Json(live)).into_response())
    }.await)
}

async fn list_requests(State(s): St, p: P, Path(id): Path<String>) -> Response {
    both(async {
        agent_space(s.as_ref(), &who(p), &id, false).await?;
        table(s.as_ref(), MERGES, &MERGE_COLUMNS).await?;
        let rows = query_platform_table(s.as_ref(), MERGES, Some(format!("agent_id = {}", sql_quote(&id))), 10_000).await.map_err(internal)?;
        ok(json!({ "merge_requests": rows.iter().map(request_json).collect::<Vec<_>>() }))
    }.await)
}

async fn get_request(State(s): St, p: P, Path((id, mr)): Path<(String, String)>) -> Response {
    both(async {
        let (_, sp) = agent_space(s.as_ref(), &who(p), &id, false).await?;
        ok(with_live_state(s.as_ref(), &sp, load(s.as_ref(), &id, &mr).await?).await?)
    }.await)
}

fn must_be_open(v: &Value) -> Result<(), Response> {
    if col(v, "status") == "open" {
        Ok(())
    } else {
        Err((StatusCode::CONFLICT, Json(json!({ "error": format!("this merge request is {}", col(v, "status")), "code": "merge_not_open" }))).into_response())
    }
}

/// `{resolutions?: [{lineage, memory, take, value?}], flow_resolutions?: [{namespace, name,
/// take, version?}]}`: replaces what was given. `take` is ADB's for memory (`branch`,
/// `main`, `edit`), and `branch`, `prod` or `edit` for a flow.
async fn set_resolutions(State(s): St, p: P, Path((id, mr)): Path<(String, String)>, Json(b): Json<Value>) -> Response {
    both(async {
        let principal = who(p);
        let (_, sp) = agent_space(s.as_ref(), &principal, &id, true).await?;
        let mut v = load(s.as_ref(), &id, &mr).await?;
        must_be_open(&v)?;
        for k in ["resolutions", "flow_resolutions"] {
            if let Some(list) = b.get(k).filter(|x| x.is_array()) {
                v[k] = list.clone();
            }
        }
        // An edit must name a flow version that exists.
        for r in v["flow_resolutions"].as_array().cloned().unwrap_or_default() {
            if col(&r, "take") == "edit" && s.workflows().get_version(&col(&r, "namespace"), &col(&r, "name"), &col(&r, "version")).await.is_err() {
                return Err(err(StatusCode::BAD_REQUEST, format!("{}/{} has no version {:?}", col(&r, "namespace"), col(&r, "name"), col(&r, "version"))));
            }
        }
        save(s.as_ref(), &v, principal.id()).await?;
        ok(with_live_state(s.as_ref(), &sp, v).await?)
    }.await)
}

/// Whether `who` may approve a request `author` opened: not your own, when the caller is a
/// person or an agent. A service has no identity to separate (CE with no sign-in).
pub fn may_approve(who: &Principal, author: &str) -> bool {
    matches!(who, Principal::Service { .. }) || who.id() != author
}

/// `{note?}`: one approval per person.
async fn approve(State(s): St, p: P, Path((id, mr)): Path<(String, String)>, body: Option<Json<Value>>) -> Response {
    both(async {
        let principal = who(p);
        let (_, sp) = agent_space(s.as_ref(), &principal, &id, true).await?;
        allowed(&principal, flowr_policy::actions::MEMORY_MERGE, &id).await?;
        let mut v = load(s.as_ref(), &id, &mr).await?;
        must_be_open(&v)?;
        if !may_approve(&principal, &col(&v, "created_by")) {
            return Err((StatusCode::FORBIDDEN, Json(json!({ "error": "you opened this merge request, so someone else has to approve it", "code": "self_approval" }))).into_response());
        }
        let note = body.map(|Json(b)| col(&b, "note")).unwrap_or_default();
        let list = v["approvals"].as_array_mut().ok_or_else(|| internal("approvals is not a list".into()))?;
        if !list.iter().any(|a| col(a, "by") == principal.id()) || matches!(principal, Principal::Service { .. }) {
            list.push(json!({ "by": principal.id(), "note": note, "at": chrono::Utc::now().to_rfc3339() }));
        }
        save(s.as_ref(), &v, principal.id()).await?;
        ok(with_live_state(s.as_ref(), &sp, v).await?)
    }.await)
}

/// `{kind, ref, note?}`: what shows it works, such as a test run on the branch.
async fn add_evidence(State(s): St, p: P, Path((id, mr)): Path<(String, String)>, Json(b): Json<Value>) -> Response {
    both(async {
        let principal = who(p);
        let (_, sp) = agent_space(s.as_ref(), &principal, &id, true).await?;
        let mut v = load(s.as_ref(), &id, &mr).await?;
        must_be_open(&v)?;
        if col(&b, "ref").is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "evidence needs a ref, such as a run id"));
        }
        v["evidence"].as_array_mut().ok_or_else(|| internal("evidence is not a list".into()))?.push(json!({
            "kind": col(&b, "kind"), "ref": col(&b, "ref"), "note": col(&b, "note"), "by": principal.id(), "at": chrono::Utc::now().to_rfc3339(),
        }));
        save(s.as_ref(), &v, principal.id()).await?;
        ok(with_live_state(s.as_ref(), &sp, v).await?)
    }.await)
}

// ── Apply and roll back (AC-10, AC-11a) ─────────────────────────────────────

/// `{new_agent?: {id?, name?}}` (kind 1). Refused with `merge_blocked` and what's in the way
/// until the gate opens.
async fn apply(State(s): St, p: P, Path((id, mr)): Path<(String, String)>, body: Option<Json<Value>>) -> Response {
    both(async {
        let principal = who(p);
        let s = s.as_ref();
        let (agent, sp) = agent_space(s, &principal, &id, true).await?;
        let stored = load(s, &id, &mr).await?;
        must_be_open(&stored)?;
        let target = environments::find(s, &project_of(&agent), &col(&stored, "environment")).await?;
        let production = environments::is_production(&target);
        let deploy = col(&stored, "kind") == "deploy";
        if deploy && production {
            allowed(&principal, flowr_policy::actions::DEPLOY_PROD, &id).await?;
        } else if !deploy {
            allowed(&principal, flowr_policy::actions::MEMORY_MERGE, &id).await?;
        }
        let mut v = with_live_state(s, &sp, stored).await?;
        let blocked = blocked_by(&v);
        if !blocked.is_empty() {
            return Err((StatusCode::CONFLICT, Json(json!({ "error": format!("this merge can't apply yet: {}", blocked.join("; ")), "code": "merge_blocked", "blocked_by": blocked }))).into_response());
        }
        let branch = col(&v, "branch");
        let main_offset = v["main"]["offset"].clone();
        let resolutions = v["resolutions"].clone();
        let result = if deploy {
            json!({ "pin": write_pin(s, &id, &col(&v, "environment"), &col(&v, "version"), principal.id()).await? })
        } else if col(&v, "kind") == "2" {
            let r = adb(s, &sp, "branch_merge", json!({ "name": branch, "memories": ["PROCEDURAL"], "main_offset": main_offset, "resolutions": resolutions, "merged_by": principal.id() })).await?;
            json!({ "merge_id": r.get("merge_id"), "adb": r })
        } else {
            adb(s, &sp, "branch_rebase", json!({ "name": branch, "main_offset": main_offset, "resolutions": resolutions, "merged_by": principal.id() })).await?;
            // The flows the new agent runs, each conflict settled as chosen.
            let chosen = v["flow_resolutions"].as_array().cloned().unwrap_or_default();
            let mut flows = Vec::new();
            for f in v["flows"].as_array().cloned().unwrap_or_default() {
                let res = chosen.iter().find(|r| col(r, "namespace") == col(&f, "namespace") && col(r, "name") == col(&f, "name"));
                let version = resolved_version(&f, res).map_err(|e| err(StatusCode::BAD_REQUEST, e))?.unwrap_or_default();
                flows.push(json!({ "namespace": col(&f, "namespace"), "name": col(&f, "name"), "version": version }));
            }
            let b = body.map(|Json(b)| b).unwrap_or_else(|| json!({}));
            let created = copy_new_agent(s, &principal, &id, &b, "branch_promote", json!({ "name": branch }), json!(flows)).await?;
            let new_id = created["agent"]["id"].as_str().unwrap_or_default().to_string();
            let moved = move_triggers(s, &id, &new_id).await?;
            json!({ "promoted_agent": new_id, "agent": created, "flows": flows, "moved_triggers": moved })
        };
        v["result"] = result;
        v["status"] = json!("merged");
        save(s, &v, principal.id()).await?;
        ok(v)
    }.await)
}

/// `{force_skip?}` (kind 2). Kind 2 reverts by merge id; kind 1 moves the triggers back to
/// the old agent, which still has its memory and conversations. The new agent stays.
async fn rollback(State(s): St, p: P, Path((id, mr)): Path<(String, String)>, body: Option<Json<Value>>) -> Response {
    both(async {
        let principal = who(p);
        let s = s.as_ref();
        let (_, sp) = agent_space(s, &principal, &id, true).await?;
        allowed(&principal, flowr_policy::actions::MEMORY_MERGE, &id).await?;
        let mut v = load(s, &id, &mr).await?;
        if col(&v, "status") != "merged" {
            return Err((StatusCode::CONFLICT, Json(json!({ "error": format!("this merge request is {}", col(&v, "status")), "code": "merge_not_merged" }))).into_response());
        }
        let undone = if col(&v, "kind") == "deploy" {
            // The version before; a first pin has none to go back to.
            let previous = v["result"]["pin"]["previous"].as_str().unwrap_or_default().to_string();
            if previous.is_empty() {
                return Err((StatusCode::CONFLICT, Json(json!({ "error": "this was the environment's first version: there's none to go back to", "code": "no_previous" }))).into_response());
            }
            json!({ "pin": write_pin(s, &id, &col(&v, "environment"), &previous, principal.id()).await? })
        } else if col(&v, "kind") == "2" {
            let force = body.and_then(|Json(b)| b.get("force_skip").and_then(|x| x.as_bool())).unwrap_or(false);
            adb(s, &sp, "merge_revert", json!({ "merge_id": v["result"]["merge_id"], "reverted_by": principal.id(), "force_skip": force })).await?
        } else {
            let new_id = v["result"]["promoted_agent"].as_str().unwrap_or_default().to_string();
            json!({ "moved_triggers": move_triggers(s, &new_id, &id).await? })
        };
        v["result"]["rollback"] = undone;
        v["status"] = json!("rolled_back");
        save(s, &v, principal.id()).await?;
        ok(v)
    }.await)
}

/// Point every trigger aimed at agent `from` at agent `to`. The ids of those moved.
async fn move_triggers(s: &dyn StorageBackend, from: &str, to: &str) -> Result<Vec<String>, Response> {
    let mut moved = Vec::new();
    for t in s.triggers().list(None).await.map_err(|e| internal(e.to_string()))? {
        let aimed = t.get("targets").and_then(|x| x.as_array()).is_some_and(|ts| ts.iter().any(|x| col(x, "type") == "agent" && col(x, "agent_id") == from));
        if aimed {
            let trigger = col(&t, "id");
            s.triggers().attach(&trigger, json!({ "type": "agent", "agent_id": to })).await.map_err(|e| internal(e.to_string()))?;
            s.triggers().detach(&trigger, from).await.map_err(|e| internal(e.to_string()))?;
            moved.push(trigger);
        }
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flow_conflicts_only_when_both_sides_changed_it_differently() {
        assert_eq!(flow_state("1", "1", "1"), FlowState::Unchanged);
        assert_eq!(flow_state("1", "2", "1"), FlowState::Unchanged, "only prod moved: prod stays");
        assert_eq!(flow_state("1", "1", "2"), FlowState::Clean);
        assert_eq!(flow_state("1", "2", "2"), FlowState::Same);
        assert_eq!(flow_state("1", "2", "3"), FlowState::Conflict);
    }

    fn flow(base: &str, prod: &str, branch: &str) -> Value {
        json!({ "namespace": "t", "name": "f", "base_version": base, "prod_version": prod, "branch_version": branch })
    }

    #[test]
    fn a_resolution_picks_the_version() {
        let c = flow("1", "2", "3");
        assert_eq!(resolved_version(&c, None).unwrap(), None);
        assert_eq!(resolved_version(&c, Some(&json!({ "take": "branch" }))).unwrap().as_deref(), Some("3"));
        assert_eq!(resolved_version(&c, Some(&json!({ "take": "prod" }))).unwrap().as_deref(), Some("2"));
        assert_eq!(resolved_version(&c, Some(&json!({ "take": "edit", "version": "4" }))).unwrap().as_deref(), Some("4"));
        assert!(resolved_version(&c, Some(&json!({ "take": "edit" }))).is_err());
        assert!(resolved_version(&c, Some(&json!({ "take": "nonsense" }))).is_err());
        assert_eq!(resolved_version(&flow("1", "2", "1"), None).unwrap().as_deref(), Some("2"), "the branch never changed it");
    }

    #[test]
    fn the_gate_lists_everything_in_the_way() {
        let mut v = json!({
            "conflicts": [{ "lineage": "l1", "key": "refund" }], "resolutions": [],
            "flows": [flow("1", "2", "3")], "flow_resolutions": [],
            "approvals": [], "evidence": [], "required_approvals": "1", "required_evidence": "1",
        });
        let b = blocked_by(&v);
        assert_eq!(b.len(), 4, "{b:?}");
        v["resolutions"] = json!([{ "lineage": "l1", "take": "main" }]);
        v["flow_resolutions"] = json!([{ "namespace": "t", "name": "f", "take": "branch" }]);
        assert_eq!(blocked_by(&v).len(), 2);
        v["approvals"] = json!([{ "by": "bob" }]);
        v["evidence"] = json!([{ "ref": "run-1" }]);
        assert!(blocked_by(&v).is_empty());
    }

    #[test]
    fn a_person_cannot_approve_their_own_merge() {
        let alice = Principal::human("alice", vec![]);
        assert!(!may_approve(&alice, "alice"));
        assert!(may_approve(&Principal::human("bob", vec![]), "alice"));
        assert!(may_approve(&Principal::service("anonymous"), "anonymous"), "no sign-in: nobody to separate");
    }
}
