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
    Router::new().route("/security/agents", get(inventory)).with_state(storage)
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
