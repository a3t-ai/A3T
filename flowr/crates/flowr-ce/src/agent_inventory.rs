//! The agent inventory (GOV-10): every agent, with who owns it, what it can
//! call and reach, its keys, when it last did something, and a risk tier — for
//! the people who answer for an org's agents. `GET /security/agents?org=&format=csv`.
//!
//! Platform InfoSec sees every org (or the one named); an org's admins and
//! security admins see their org's agents. The risk tier is a rule of thumb,
//! said in the row's `why`:
//! - **high** — it can call tools on servers outside A3T or tools that delete,
//!   with no rule of its own and no egress list; or a live key with no expiry;
//! - **medium** — it reaches outside A3T or has live keys, with limits set;
//! - **low** — A3T's own tools only, no live keys.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::Path,
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Extension, Json, Router,
};
use serde_json::{json, Value};

use flowr_policy::Principal;
use flowr_storage::StorageBackend;

use crate::handlers::call_lancedb::query_platform_table;

pub fn routes(storage: Arc<dyn StorageBackend>) -> Router {
    Router::new()
        .route("/security/agents", get(inventory))
        // One agent as security sees it: identity, keys, effective access, changes.
        .route("/security/agents/:id", get(one_agent))
        .route("/security/agents/:id/keys/revoke", axum::routing::post(revoke_keys))
        .with_state(storage)
}

fn refuse(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

/// The agent's row, if `p` manages the security of its org (or is platform InfoSec).
async fn security_view(s: &dyn StorageBackend, p: &Principal, id: &str) -> Result<Value, Response> {
    let row = query_platform_table(s, "agent_configs", Some(format!("id = {}", crate::handlers::call_lancedb::sql_quote(id))), 1).await
        .ok().and_then(|r| r.into_iter().next())
        .ok_or_else(|| refuse(StatusCode::NOT_FOUND, format!("no agent {id:?}")))?;
    let infosec = p.roles().iter().any(|r| r == flowr_policy::roles::SUPER_ADMIN || r == flowr_policy::roles::SECURITY_ADMIN);
    if !infosec && !crate::org_policy::manages_org_security(s, p, &col(&row, "org_id")).await {
        // Same answer as a missing agent: ids can't be probed.
        return Err(refuse(StatusCode::NOT_FOUND, format!("no agent {id:?}")));
    }
    Ok(row)
}

/// What a decision says, for a row of the access table.
fn verdict(d: &flowr_policy::PolicyDecision) -> Value {
    match d {
        flowr_policy::PolicyDecision::Allow => json!({ "state": "allowed" }),
        flowr_policy::PolicyDecision::Deny { reason, policy_ids } => json!({ "state": "blocked", "by": policy_ids, "why": reason }),
        flowr_policy::PolicyDecision::Ask { reason, policy_ids, required, .. } => json!({
            "state": "needs approval", "by": policy_ids, "why": reason,
            "approver": required.as_ref().and_then(|r| r.approver.clone()) }),
    }
}

/// `GET /security/agents/{id}`: the inventory row, its keys (never the
/// secret), who it acts for is in the decision log; its effective access —
/// each server, tool, model, host and memory it is bound to or reaches,
/// decided by the policy hook as the agent acting for its owner (a what-if:
/// nothing logged or counted); and what changed (its record and its policy).
async fn one_agent(State(s): State<Arc<dyn StorageBackend>>, p: Option<Extension<Principal>>, Path(id): Path<String>) -> Response {
    let Some(Extension(p)) = p else { return refuse(StatusCode::UNAUTHORIZED, "Sign in first.") };
    let s = s.as_ref();
    let row = match security_view(s, &p, &id).await { Ok(r) => r, Err(r) => return r };
    let keys = crate::agent_keys::list(s, &id).await.unwrap_or_default();
    let policy_row = crate::agent_space::security::policy_of(s, &id).await;
    let policy = policy_row.as_ref().map(|r| col(r, "policy"));
    let events = query_platform_table(s, "agent_events", Some(format!("agent_config_id = {}", crate::handlers::call_lancedb::sql_quote(&id))), 1000).await.unwrap_or_default();
    let last = events.iter().map(|e| col(e, "created_at")).max();
    let agent = row_of(&row, &keys.iter().map(|k| json!({ "revoked_at": k["revoked_at"], "expires_at": k["expires_at"] })).collect::<Vec<_>>(), policy.as_deref(), last.as_deref());
    let project = col(&row, "namespace");
    let owner = col(&row, "created_by");

    // Effective access: as the agent, acting for its owner, in its project.
    let own = policy.as_deref().and_then(|y| flowr_policy::policy_file::PolicyFile::parse(y).ok());
    let ceiling = own.as_ref().and_then(|f| f.role_ceiling.clone());
    let human = Principal::human(owner.clone(), vec![flowr_policy::roles::ADMIN.into(), flowr_policy::roles::OPERATOR.into(), flowr_policy::roles::VIEWER.into()]);
    let as_agent = Principal::agent_for(flowr_policy::AgentId(id.clone()), &human, None, Some(owner.clone()), ceiling.as_deref());
    let hook = flowr_policy::policy_hook();
    let probe = |action: flowr_policy::Action, resource: flowr_policy::Resource| {
        let mut attributes = serde_json::Map::new();
        attributes.insert("_probe".into(), json!(true));
        flowr_policy::ExecutionRequest { principal: as_agent.clone(), action, resource, context: Default::default(), attributes,
            params: json!({ "namespace": project }) }
    };
    let mut access = Vec::new();
    for srv in strings(&agent["mcp_servers"]) {
        let d = hook.check(&probe(flowr_policy::actions::mcp_call(), flowr_policy::Resource::tool(format!("{srv}/*")).with_attr("server", srv.clone()))).await;
        access.push(json!({ "kind": "MCP server", "what": srv, "decision": verdict(&d) }));
    }
    for t in strings(&agent["tools"]) {
        let d = hook.check(&probe(flowr_policy::actions::mcp_call(), flowr_policy::Resource::tool(t.clone()))).await;
        access.push(json!({ "kind": "Tool", "what": t, "decision": verdict(&d) }));
    }
    let model = agent["model"].as_str().map(str::to_string).or_else(|| agent["model"].get("name").and_then(|v| v.as_str()).map(str::to_string));
    if let Some(m) = model.filter(|m| !m.is_empty()) {
        let d = hook.check(&probe(flowr_policy::actions::llm_call(), flowr_policy::Resource::new("Llm", m.clone()).with_attr("name", m.clone()))).await;
        access.push(json!({ "kind": "Model", "what": m, "decision": verdict(&d) }));
    }
    // What its latest version reaches (flows, layers): hosts, memory, subflows.
    let reach = match crate::agent_space::version_rows(s, &id).await.ok().and_then(|r| r.last().map(crate::agent_space::version_json)) {
        Some(v) => crate::agent_space::manifest::of_version(s, &v).await,
        None => Value::Null,
    };
    for (k, kind) in [("hosts", "Outside service"), ("memory", "Memory"), ("subflows", "Flow it runs")] {
        for x in strings(&reach[k]) {
            access.push(json!({ "kind": kind, "what": x, "decision": { "state": "reached", "why": "its flows reach it; checked when called" } }));
        }
    }
    if let Some(e) = own.as_ref().and_then(|f| f.egress.clone()) {
        for h in e {
            access.push(json!({ "kind": "Outside service", "what": h, "decision": { "state": "allowed", "why": "on its own egress list" } }));
        }
    }

    // What changed: its record (agent_events) and its own policy (history).
    let mut changes: Vec<Value> = events.iter().filter(|e| col(e, "event_type") != "turn").map(|e| {
        let payload: Value = serde_json::from_str(&col(e, "payload")).unwrap_or(Value::Null);
        json!({ "at": col(e, "created_at"), "what": col(e, "event_type"), "by": payload.get("by").cloned().unwrap_or(Value::Null),
            "approved_by": payload.get("approved_by").cloned().unwrap_or(Value::Null), "changed": payload.get("changed").cloned().unwrap_or(Value::Null) })
    }).collect();
    for h in crate::agent_space::security::history_of(s, &id).await {
        changes.push(json!({ "at": col(&h, "at"), "what": format!("policy {}", col(&h, "action")), "by": col(&h, "by"), "version": col(&h, "version") }));
    }
    for k in &keys {
        changes.push(json!({ "at": k["created_at"], "what": "key created", "by": k["created_by"], "key": k["name"] }));
        if k["revoked_at"].as_str().is_some_and(|r| !r.is_empty()) {
            changes.push(json!({ "at": k["revoked_at"], "what": "key revoked", "key": k["name"] }));
        }
    }
    changes.sort_by(|a, b| b["at"].as_str().unwrap_or("").cmp(a["at"].as_str().unwrap_or("")));
    changes.truncate(200);

    Json(json!({
        "agent": agent, "keys": keys, "access": access, "changes": changes,
        "acts_as": { "own_key": "its keys act as the person who made them, within its ceiling", "ceiling": ceiling,
            "outside": "with its own key: the caller's identity isn't passed on to outside services" },
        "policy": { "set": own.is_some(), "rules": own.as_ref().map(|f| f.rules.iter().map(flowr_policy::policy_file::describe_rule).collect::<Vec<_>>()).unwrap_or_default() },
    })).into_response()
}

/// `POST /security/agents/{id}/keys/revoke`: revoke every live key of the agent.
async fn revoke_keys(State(s): State<Arc<dyn StorageBackend>>, p: Option<Extension<Principal>>, Path(id): Path<String>) -> Response {
    let Some(Extension(p)) = p else { return refuse(StatusCode::UNAUTHORIZED, "Sign in first.") };
    let s = s.as_ref();
    if let Err(r) = security_view(s, &p, &id).await { return r; }
    let keys = crate::agent_keys::list(s, &id).await.unwrap_or_default();
    let mut revoked = 0;
    for k in keys.iter().filter(|k| k["revoked_at"].as_str().unwrap_or("").is_empty()) {
        match crate::agent_keys::revoke(s, &id, k["id"].as_str().unwrap_or_default()).await {
            Ok(true) => revoked += 1,
            Ok(false) => {}
            Err(e) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, e),
        }
    }
    flowr_policy::audit::event(&p, "agent:keys_revoke", "Agent", &id, None, None, None, Some(&format!("{revoked} key(s) revoked from Security")));
    Json(json!({ "agent_id": id, "revoked": revoked })).into_response()
}

fn col(row: &Value, k: &str) -> String {
    row.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}
fn parsed(row: &Value, k: &str) -> Value {
    match row.get(k) {
        Some(Value::String(t)) => serde_json::from_str(t).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    }
}
fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// One agent's row.
pub fn row_of(agent: &Value, keys: &[Value], policy: Option<&str>, last: Option<&str>) -> Value {
    let vars = parsed(agent, "variables");
    let binds = parsed(agent, "resource_bindings");
    let pick = |k: &str| { let b = binds.get(k).cloned().unwrap_or(Value::Null); if b.is_null() { vars.get(k).cloned().unwrap_or(Value::Null) } else { b } };
    let mcp = strings(&pick("mcp_servers"));
    let tools = pick("enabled_tools");
    let tool_list = strings(&tools);
    let external: Vec<&String> = mcp.iter().filter(|s| !s.starts_with("a3t/") && !s.starts_with("system/")).collect();
    let deletes = tool_list.iter().any(|t| t.contains("delete") || t.contains("drop") || t.contains("forget")) || (tools.is_null() && !mcp.is_empty());
    let now = chrono::Utc::now();
    let live: Vec<&Value> = keys.iter().filter(|k| col(k, "revoked_at").is_empty()
        && chrono::DateTime::parse_from_rfc3339(&col(k, "expires_at")).map(|t| t > now).unwrap_or(true)).collect();
    let no_expiry = live.iter().any(|k| col(k, "expires_at").is_empty());
    let own = policy.and_then(|p| flowr_policy::policy_file::PolicyFile::parse(p).ok());
    let has_rules = own.as_ref().is_some_and(|f| !f.rules.is_empty() || f.cedar.is_some());
    let egress = own.as_ref().and_then(|f| f.egress.clone());
    let mut why = Vec::new();
    let tier = if (!external.is_empty() || deletes) && !has_rules && egress.is_none() {
        why.push(if !external.is_empty() { format!("calls tools on {} with no rule of its own", external.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")) } else { "can call tools that delete, with no rule of its own".into() });
        "high"
    } else if no_expiry {
        why.push("a live key that never expires".into());
        "high"
    } else if !external.is_empty() || !live.is_empty() {
        if !external.is_empty() { why.push(format!("reaches {}", external.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))); }
        if !live.is_empty() { why.push(format!("{} live key(s)", live.len())); }
        "medium"
    } else {
        why.push("A3T's own tools only; no live keys".into());
        "low"
    };
    json!({
        "id": col(agent, "id"), "name": col(agent, "name"), "project": col(agent, "namespace"), "org": col(agent, "org_id"),
        "owner": col(agent, "created_by"), "status": col(agent, "status"), "template": col(agent, "template_id"),
        "model": pick("llm"), "mcp_servers": mcp, "tools": if tools.is_null() { json!("any its servers offer") } else { tools },
        "keys_live": live.len(), "keys_total": keys.len(),
        "own_policy": policy.is_some(), "rules": own.as_ref().map(|f| f.rules.len()).unwrap_or(0),
        "role_ceiling": own.as_ref().and_then(|f| f.role_ceiling.clone()), "egress": egress,
        "last_activity": last, "created_at": col(agent, "created_at"),
        "risk": tier, "why": why.join("; "),
    })
}

async fn inventory(State(s): State<Arc<dyn StorageBackend>>, p: Option<Extension<Principal>>, Query(q): Query<HashMap<String, String>>) -> Response {
    let Some(Extension(p)) = p else { return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "Sign in first." }))).into_response() };
    let s = s.as_ref();
    let infosec = p.roles().iter().any(|r| r == flowr_policy::roles::SUPER_ADMIN || r == flowr_policy::roles::SECURITY_ADMIN);
    let want_org = q.get("org").filter(|o| !o.is_empty()).cloned();
    let agents = query_platform_table(s, "agent_configs", None, 100_000).await.unwrap_or_default();
    let keys = query_platform_table(s, crate::agent_keys::TABLE, None, 100_000).await.unwrap_or_default();
    let policies = query_platform_table(s, crate::agent_space::security::TABLE, None, 100_000).await.unwrap_or_default();
    let events = query_platform_table(s, "agent_events", None, 100_000).await.unwrap_or_default();
    let mut last: HashMap<String, String> = HashMap::new();
    for e in &events {
        let (id, at) = (col(e, "agent_config_id"), col(e, "created_at"));
        if last.get(&id).is_none_or(|l| *l < at) { last.insert(id, at); }
    }
    let mut out = Vec::new();
    let mut allowed: HashMap<String, bool> = HashMap::new();
    for a in &agents {
        let org = col(a, "org_id");
        if want_org.as_ref().is_some_and(|w| *w != org) { continue; }
        let ok = if infosec { true } else {
            match allowed.get(&org) {
                Some(v) => *v,
                None => { let v = crate::org_policy::manages_org_security(s, &p, &org).await; allowed.insert(org.clone(), v); v }
            }
        };
        if !ok { continue; }
        let id = col(a, "id");
        let ks: Vec<Value> = keys.iter().filter(|k| col(k, "agent_id") == id).cloned().collect();
        let pol = policies.iter().find(|r| col(r, "agent_id") == id).map(|r| col(r, "policy"));
        out.push(row_of(a, &ks, pol.as_deref(), last.get(&id).map(|s| s.as_str())));
    }
    let rank = |r: &Value| match r["risk"].as_str() { Some("high") => 0, Some("medium") => 1, _ => 2 };
    out.sort_by(|a, b| rank(a).cmp(&rank(b)).then(col(a, "org").cmp(&col(b, "org"))).then(col(a, "name").cmp(&col(b, "name"))));
    if q.get("format").map(|f| f == "csv").unwrap_or(false) {
        let cols = ["id", "name", "org", "project", "owner", "status", "risk", "why", "model", "mcp_servers", "tools", "keys_live", "own_policy", "rules", "egress", "last_activity", "created_at"];
        let cell = |v: &Value| -> String {
            let t = match v { Value::String(s) => s.clone(), Value::Null => String::new(), Value::Array(a) => a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join(" "), o => o.to_string() };
            format!("\"{}\"", t.replace('"', "\"\""))
        };
        let mut csv = cols.join(",") + "\n";
        for r in &out { csv.push_str(&cols.iter().map(|c| cell(&r[*c])).collect::<Vec<_>>().join(",")); csv.push('\n'); }
        return ([(header::CONTENT_TYPE, "text/csv; charset=utf-8"), (header::CONTENT_DISPOSITION, "attachment; filename=\"a3t-agents.csv\"")], csv).into_response();
    }
    let count = |t: &str| out.iter().filter(|r| r["risk"] == t).count();
    Json(json!({ "count": out.len(), "high": count("high"), "medium": count("medium"), "low": count("low"), "agents": out,
        "scope": if infosec { "every org" } else { "the orgs whose security you manage" } })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_tiers_follow_reach_rules_and_keys() {
        let agent = |mcp: &str, tools: Value| json!({ "id": "a", "name": "a", "variables": json!({ "mcp_servers": [mcp], "enabled_tools": tools }).to_string() });
        assert_eq!(row_of(&agent("acme/github", json!(["merge_pr"])), &[], None, None)["risk"], "high");
        assert_eq!(row_of(&agent("acme/github", json!(["merge_pr"])), &[], Some("kind: a3t/policy\nname: p\negress: [api.github.com]\n"), None)["risk"], "medium");
        assert_eq!(row_of(&agent("a3t/agent-flows", json!(["delete_flow"])), &[], None, None)["risk"], "high");
        assert_eq!(row_of(&agent("a3t/agent-flows", json!(["list_flows"])), &[], None, None)["risk"], "low");
        let key = json!({ "revoked_at": "", "expires_at": "" });
        assert_eq!(row_of(&agent("a3t/agent-flows", json!(["list_flows"])), &[key], None, None)["risk"], "high");
    }
}
