//! An agent's security profile: its own policy (what it may not do, wherever it
//! runs) and, in plain words, what it can reach and what limits it.
//!
//! The policy is a `kind: a3t/policy` file with no `applies_to`: it applies to
//! the agent itself. It is compiled and checked when it is saved (FlowR EE's
//! policy gate), reloaded on every FlowR, and copied when the agent is cloned,
//! forked, promoted or copied into another project, so a buyer gets the agent
//! with its limits; the buyer's own org policies can narrow it further, never
//! widen it.
//!
//! Stored in `agent_policies` until agents live in git (approved 16 Nov 2026).
//!
//! - `GET /agents/{id}/policy` · `PUT` `{policy}` · `DELETE`
//! - `GET /agents/{id}/security`: the profile.

use super::*;

pub const TABLE: &str = "agent_policies";
const COLUMNS: [&str; 4] = ["agent_id", "policy", "updated_by", "updated_at"];
/// Every change to an agent's policy, kept: who, when, what it became.
pub const HISTORY: &str = "agent_policy_history";
const HISTORY_COLUMNS: [&str; 6] = ["agent_id", "version", "action", "policy", "by", "at"];

/// An agent's limits are security: only its org's admins and security admins
/// (or platform InfoSec) change them, not the project's writers whose agent it is.
async fn may_change_policy(s: &dyn StorageBackend, principal: &Principal, row: &Value) -> Result<(), Response> {
    if crate::org_policy::manages_org_security(s, principal, &col(row, "org_id")).await {
        Ok(())
    } else {
        Err(err(StatusCode::FORBIDDEN, "an agent's policy is changed by its org's admins or security admins"))
    }
}

async fn history_of(s: &dyn StorageBackend, id: &str) -> Vec<Value> {
    let mut h = query_platform_table(s, HISTORY, Some(format!("agent_id = {}", sql_quote(id))), 10_000).await.unwrap_or_default();
    h.sort_by_key(|r| std::cmp::Reverse(col(r, "version").trim_start_matches('v').parse::<u64>().unwrap_or(0)));
    h
}

/// Record a change; returns its version (`v<n>`, never reused).
async fn note(s: &dyn StorageBackend, id: &str, action: &str, policy: &str, by: &str) -> Result<String, String> {
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let schema: Map<String, Value> = HISTORY_COLUMNS.iter().map(|c| (c.to_string(), json!("string"))).collect();
    db.create_table(HISTORY, Some(&schema), "create_if_not_exists").await?;
    let last = history_of(s, id).await.first().map(|r| col(r, "version").trim_start_matches('v').parse::<u64>().unwrap_or(0)).unwrap_or(0);
    let version = format!("v{}", last + 1);
    db.insert(HISTORY, &[json!({
        "agent_id": id, "version": version, "action": action, "policy": policy, "by": by,
        "at": chrono::Utc::now().to_rfc3339(),
    })]).await?;
    tracing::info!(agent = %id, %version, %action, %by, "agent policy changed");
    Ok(version)
}

/// The agent row, if the caller may see it (`write`: change its project).
async fn agent_for(s: &dyn StorageBackend, principal: &Principal, id: &str, write: bool) -> Result<Value, Response> {
    let row = agent_row(s, id).await?;
    let project = project_of(&row);
    if write {
        crate::access::rest_write(principal, &project).await.map_err(IntoResponse::into_response)?;
    } else if !crate::access::can_read_item(principal, ItemKind::Agent, &project, &col(&row, "name"), "").await
        // Its org's security managers see its security profile, member of its project or not.
        && !crate::org_policy::manages_org_security(s, principal, &col(&row, "org_id")).await
    {
        return Err(err(StatusCode::NOT_FOUND, format!("no agent {id:?}")));
    }
    Ok(row)
}

async fn table(s: &dyn StorageBackend) -> Result<std::sync::Arc<dyn crate::platform_db::PlatformDb>, String> {
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let schema: Map<String, Value> = COLUMNS.iter().map(|c| (c.to_string(), json!("string"))).collect();
    db.create_table(TABLE, Some(&schema), "create_if_not_exists").await?;
    Ok(db)
}

/// Every agent's own policy: `(agent_id, project, policy yaml)`. FlowR EE
/// compiles these into its policy set.
pub async fn all_policies(s: &dyn StorageBackend) -> Vec<(String, String, String)> {
    let rows = query_platform_table(s, TABLE, None, 100_000).await.unwrap_or_default();
    let mut out = Vec::new();
    for r in rows {
        let id = col(&r, "agent_id");
        let policy = col(&r, "policy");
        if id.is_empty() || policy.trim().is_empty() {
            continue;
        }
        // The agent's project decides which projects' policy sets carry it.
        let project = match agent_row(s, &id).await {
            Ok(row) => project_of(&row),
            Err(_) => continue, // the agent is gone; its policy goes with it
        };
        out.push((id, project, policy));
    }
    out
}

async fn policy_of(s: &dyn StorageBackend, id: &str) -> Option<Value> {
    query_platform_table(s, TABLE, Some(format!("agent_id = {}", sql_quote(id))), 1).await.ok()?.into_iter().next()
}

/// The rules of a policy file in plain words (empty when it doesn't parse).
/// The file applies to this agent alone, so a rule naming nobody reads as
/// "This agent may not …" rather than "Nobody may …".
fn sentences(yaml: &str) -> Vec<String> {
    flowr_policy::policy_file::PolicyFile::parse(yaml)
        .map(|f| f.rules.iter().map(|r| {
            let s = flowr_policy::policy_file::describe_rule(r);
            match s.strip_prefix("Nobody may ") {
                Some(rest) => format!("This agent may not {rest}"),
                None => s,
            }
        }).collect())
        .unwrap_or_default()
}

/// Copy an agent's policy to a new agent (clone, fork, promote, copy to a project).
pub(super) async fn copy_policy(s: &dyn StorageBackend, from: &str, to: &str, by: &str) -> Result<(), String> {
    let Some(row) = policy_of(s, from).await else { return Ok(()) };
    let db = table(s).await?;
    db.upsert(TABLE, &["agent_id"], &[json!({
        "agent_id": to, "policy": col(&row, "policy"), "updated_by": by,
        "updated_at": chrono::Utc::now().to_rfc3339(),
    })]).await?;
    note(s, to, &format!("copy from {from}"), &col(&row, "policy"), by).await?;
    if let Some(gate) = crate::policy_gate::get() {
        gate.changed().await;
    }
    Ok(())
}

pub(super) async fn get_policy(State(s): St, p: P, Path(id): Path<String>) -> Response {
    both(async {
        let principal = who(p);
        let agent = agent_for(s.as_ref(), &principal, &id, false).await?;
        let can_edit = crate::org_policy::manages_org_security(s.as_ref(), &principal, &col(&agent, "org_id")).await;
        let row = policy_of(s.as_ref(), &id).await;
        let policy = row.as_ref().map(|r| col(r, "policy"));
        ok(json!({
            "agent_id": id,
            "policy": policy,
            "can_edit": can_edit,
            "rules": policy.as_deref().map(sentences).unwrap_or_default(),
            "updated_by": row.as_ref().map(|r| col(r, "updated_by")),
            "updated_at": row.as_ref().map(|r| col(r, "updated_at")),
            "history": history_of(s.as_ref(), &id).await.iter().map(|h| json!({
                "version": col(h, "version"), "action": col(h, "action"), "by": col(h, "by"), "at": col(h, "at"), "policy": col(h, "policy"),
            })).collect::<Vec<_>>(),
        }))
    }.await)
}

pub(super) async fn put_policy(State(s): St, p: P, Path(id): Path<String>, Json(b): Json<Value>) -> Response {
    let principal = who(p);
    let policy = b.get("policy").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    both(save_policy(s.as_ref(), &principal, &id, &policy, None).await)
}

/// The org (its slug) an agent belongs to, for its org's settings (GOV-3).
async fn org_slug(s: &dyn StorageBackend, row: &Value) -> String {
    let id = col(row, "org_id");
    if id.is_empty() {
        return String::new();
    }
    query_platform_table(s, "orgs", Some(format!("id = {}", sql_quote(&id))), 1).await.ok()
        .and_then(|r| r.into_iter().next()).map(|r| col(&r, "slug")).unwrap_or_default()
}

/// Save an agent's own policy. `approved`: the author of a change a second
/// admin approved (GOV-3); without it, an org with a second approver holds it.
pub(crate) async fn save_policy(s: &dyn StorageBackend, principal: &Principal, id: &str, policy: &str, approved: Option<&str>) -> Reply {
    let row = agent_for(s, principal, id, false).await?;
    may_change_policy(s, principal, &row).await?;
    if policy.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "policy: a kind: a3t/policy file (DELETE removes it)"));
    }
    let gate = crate::policy_gate::get()
        .ok_or_else(|| err(StatusCode::NOT_IMPLEMENTED, "agent security policies need FlowR EE"))?;
    // Checked before it is stored: a policy that doesn't compile is never saved.
    let compiled = gate.compile_agent_policy(&project_of(&row), id, policy).await
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let org = org_slug(s, &row).await;
    if approved.is_none() && !org.is_empty() && crate::policy_changes::four_eyes(s, &org).await {
        return crate::policy_changes::propose(s, principal, &org, "agent", "save", id, json!({ "policy": policy }),
            &format!("save agent {}'s policy", col(&row, "name"))).await;
    }
    let by = approved.map(str::to_string).unwrap_or_else(|| principal.id().to_string());
    let db = table(s).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    db.upsert(TABLE, &["agent_id"], &[json!({
        "agent_id": id, "policy": policy, "updated_by": by,
        "updated_at": chrono::Utc::now().to_rfc3339(),
    })]).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let version = note(s, id, "save", policy, &by).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    flowr_policy::audit::event(principal, "agent_policy:write", "AgentPolicy", id, Some(&project_of(&row)), None, None, Some(&format!("saved {version}")));
    gate.changed().await;
    ok(json!({ "agent_id": id, "policy": policy, "version": version, "rules": sentences(policy), "hidden_apps": compiled.get("hidden_apps") }))
}

pub(super) async fn delete_policy(State(s): St, p: P, Path(id): Path<String>) -> Response {
    let principal = who(p);
    both(drop_policy(s.as_ref(), &principal, &id, None).await)
}

/// Remove an agent's own policy (see [`save_policy`] for `approved`).
pub(crate) async fn drop_policy(s: &dyn StorageBackend, principal: &Principal, id: &str, approved: Option<&str>) -> Reply {
    let row = agent_for(s, principal, id, false).await?;
    may_change_policy(s, principal, &row).await?;
    let Some(prev) = policy_of(s, id).await else {
        return Err(err(StatusCode::NOT_FOUND, format!("agent {id:?} has no policy")));
    };
    let org = org_slug(s, &row).await;
    if approved.is_none() && !org.is_empty() && crate::policy_changes::four_eyes(s, &org).await {
        return crate::policy_changes::propose(s, principal, &org, "agent", "delete", id, json!({}),
            &format!("remove agent {}'s policy", col(&row, "name"))).await;
    }
    let by = approved.map(str::to_string).unwrap_or_else(|| principal.id().to_string());
    let db = table(s).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    db.delete(TABLE, &format!("agent_id = {}", sql_quote(id))).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let version = note(s, id, "delete", &col(&prev, "policy"), &by).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    flowr_policy::audit::event(principal, "agent_policy:delete", "AgentPolicy", id, Some(&project_of(&row)), None, None, Some(&format!("deleted ({version})")));
    if let Some(gate) = crate::policy_gate::get() {
        gate.changed().await;
    }
    ok(json!({ "agent_id": id, "deleted": true }))
}

/// The security profile: what the agent can reach (its latest version's
/// manifest), what it is bound to (model, MCP servers, tool ceiling), and its
/// own policy in plain words. For buyers and SecOps; no Cedar.
pub(super) async fn get_security(State(s): St, p: P, Path(id): Path<String>) -> Response {
    both(async {
        let principal = who(p);
        let row = agent_for(s.as_ref(), &principal, &id, false).await?;
        let can_edit = crate::org_policy::manages_org_security(s.as_ref(), &principal, &col(&row, "org_id")).await;
        let history: Vec<Value> = history_of(s.as_ref(), &id).await.iter().map(|h| json!({
            "version": col(h, "version"), "action": col(h, "action"), "by": col(h, "by"), "at": col(h, "at"),
        })).collect();
        let parse = |k: &str| -> Value {
            match row.get(k) {
                Some(Value::String(t)) => serde_json::from_str(t).unwrap_or(Value::Null),
                Some(v) => v.clone(),
                None => Value::Null,
            }
        };
        let bindings = parse("resource_bindings");
        let variables = parse("variables");
        let pick = |k: &str| bindings.get(k).cloned().or_else(|| variables.get(k).cloned()).unwrap_or(Value::Null);
        let latest = version_rows(s.as_ref(), &id).await.ok().and_then(|rows| rows.last().map(version_json));
        let reach = match &latest {
            Some(v) => manifest::of_version(s.as_ref(), v).await,
            None => Value::Null,
        };
        let pol = policy_of(s.as_ref(), &id).await;
        let policy = pol.as_ref().map(|r| col(r, "policy"));
        ok(json!({
            "agent_id": id,
            "project": project_of(&row),
            "version": latest.as_ref().map(|v| col(v, "version")),
            "reach": reach,
            "bound_to": {
                "model": pick("llm"),
                "mcp_servers": pick("mcp_servers"),
                // UI-3: set in its own policy (role_ceiling); absent: not narrowed.
                "role_ceiling": policy.as_deref().and_then(|y| flowr_policy::policy_file::PolicyFile::parse(y).ok()).and_then(|f| f.role_ceiling),
                "enabled_tools": pick("enabled_tools"),
            },
            "policy": {
                "set": policy.is_some(),
                "rules": policy.as_deref().map(sentences).unwrap_or_default(),
                // The file as data, for the console's rule editor.
                "file": policy.as_deref().and_then(|y| flowr_policy::policy_file::PolicyFile::parse(y).ok()),
                "source": policy,
                "updated_by": pol.as_ref().map(|r| col(r, "updated_by")),
                // Only the org's admins and security admins change it.
                "can_edit": can_edit,
                "history": history,
            },
            "note": "The agent's own policy travels with it when it is copied. The A3T baseline and the policies of the org running it apply too; they can only narrow what it may do.",
        }))
    }.await)
}
