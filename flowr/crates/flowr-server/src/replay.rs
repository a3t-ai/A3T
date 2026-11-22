//! Replay a policy change against past decisions (GOV-2).
//!
//! `POST /security/orgs/{org}/replay` `{files?: [{path, content}], removed?: [path],
//! switch_on?: [path], days?: 7, limit?: 2000}` — the org's decisions of the last
//! `days` (from the durable decision log), each rebuilt as a request and decided
//! twice: by the policies enforced now, and by the same policies with the org's
//! files changed as asked. The decisions that would flip are listed. Nothing is
//! saved or enforced.
//!
//! A decision is rebuilt from what the log keeps (who, their roles, for whom,
//! which agent, the action, the resource and its project); attributes the log
//! doesn't keep (a flow's tags, a call's arguments) are absent from both runs,
//! so a flip is the change's own effect. Events that are not policy checks
//! (approvals answered, keys made, files saved…) are skipped.

use std::sync::Arc;

use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde_json::{json, Value};

use flowr_ce::flowr_storage::StorageBackend;
use flowr_policy::{AgentId, ExecutionRequest, PolicyDecision, PolicyHook, Principal, Resource};

/// Actions written by events, not by a policy check: nothing to replay.
const EVENTS: &[&str] = &["approval:", "input:", "policy:", "agent_policy:", "key:", "auth:", "project:access", "session:"];

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string()
}

/// A logged decision, rebuilt as the request it decided.
pub fn request_of(r: &Value) -> Option<ExecutionRequest> {
    let action = flowr_policy::Action::new(&s(r, "action")).ok()?;
    let roles: Vec<String> = r.get("roles").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let opt = |k: &str| Some(s(r, k)).filter(|x| !x.is_empty());
    let principal = match s(r, "principal_kind").as_str() {
        "human" => Principal::Human { user_id: s(r, "principal_id"), roles, memory_branch: None, impersonated_by: None, session: None },
        "agent" => Principal::Agent { session: None,
            agent_id: AgentId(s(r, "principal_id")), on_behalf_of: opt("on_behalf_of"), flow_id: opt("flow"),
            roles, owner: opt("agent_owner"), memory_branch: None,
        },
        "service" => Principal::Service { service_id: s(r, "principal_id"), memory_branch: None },
        _ => return None,
    };
    let (rtype, rid) = (s(r, "resource_type"), s(r, "resource_id"));
    if rtype.is_empty() {
        return None;
    }
    let mut resource = Resource::new(&rtype, rid.clone());
    if matches!(rtype.as_str(), "Tool" | "Llm") {
        resource = resource.with_attr("name", rid.as_str());
    }
    if let Some(p) = opt("project") {
        resource.attributes.insert("namespace".into(), json!(p));
    }
    let mut attributes = serde_json::Map::new();
    if let Some(a) = opt("agent") {
        attributes.insert("template".into(), json!({ "agent": a }));
    }
    Some(ExecutionRequest { principal, action, resource, context: Default::default(), attributes, params: Value::Null })
}

fn word(d: &PolicyDecision) -> &'static str {
    d.label()
}

#[cfg(feature = "ee")]
pub async fn replay_handler(
    principal: Option<Extension<Principal>>,
    Extension(storage): Extension<Arc<dyn StorageBackend>>,
    Path(org): Path<String>,
    Json(b): Json<Value>,
) -> Response {
    let refuse = |code: StatusCode, msg: String| (code, Json(json!({ "error": msg }))).into_response();
    let Some(Extension(p)) = principal else { return refuse(StatusCode::UNAUTHORIZED, "Sign in first.".into()) };
    let st = storage.as_ref();
    if let Err(r) = flowr_ce::org_policy::allowed(st, &p, &org).await {
        return r;
    }
    if !flowr_ce::decision_log::installed() {
        return refuse(StatusCode::NOT_IMPLEMENTED, "Replaying needs the durable decision log (ADB).".into());
    }
    let days = b.get("days").and_then(|v| v.as_u64()).unwrap_or(7).clamp(1, 30);
    let limit = b.get("limit").and_then(|v| v.as_u64()).unwrap_or(2000).clamp(1, 10_000) as usize;
    let list = |k: &str| -> Vec<String> { b.get(k).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
    let drafts: Vec<(String, String)> = b.get("files").and_then(|v| v.as_array()).map(|a| a.iter()
        .map(|f| (s(f, "path"), s(f, "content"))).filter(|(p, _)| !p.is_empty()).collect()).unwrap_or_default();

    // The candidate: the org's files as they would be.
    let (projects, files) = flowr_ce::org_policy::candidate(st, &org, &drafts, &list("removed"), &list("switch_on")).await;
    let org_files: Vec<a3t_security::policy_files::OrgFile> = files.iter()
        .map(|(path, content)| a3t_security::policy_files::OrgFile { path: path.clone(), content: content.clone() }).collect();
    let set = match a3t_security::policy_files::compile_org_set(&org, &projects, &org_files) {
        Ok(set) => set,
        Err(errors) => return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": "the change doesn't compile", "errors": errors }))).into_response(),
    };
    let now_set = crate::collect_registry_cedar_policies(&storage).await;
    let mine = format!("org.{org}.");
    let mut cand_set: Vec<(String, String, String)> = now_set.iter()
        .filter(|(_, n, _)| !n.starts_with(&mine) && !n.starts_with(&format!("{}{mine}", a3t_security::SHADOW_PREFIX)) && n != &format!("org-{org}-broken"))
        .cloned().collect();
    cand_set.extend(set.policies);
    let (schema_base, fragments) = crate::collect_registry_schema(&storage).await;
    let build = |ps: &[(String, String, String)]| a3t_security::CedarHook::with_named_policies(&crate::named(ps), schema_base.as_deref(), &fragments).map(|h| h.quiet());
    let (now_hook, cand_hook) = match (build(&now_set), build(&cand_set)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return refuse(StatusCode::UNPROCESSABLE_ENTITY, format!("the policies don't load: {e}")),
    };

    // The org's decisions of the last `days`, newest first, paged.
    let Some(org_id) = flowr_ce::org_policy::org_id(st, &org).await else {
        return refuse(StatusCode::NOT_FOUND, format!("no org {org:?}"));
    };
    let since = (chrono::Utc::now() - chrono::Duration::days(days as i64)).timestamp_millis() as u64;
    let mut q = flowr_ce::decision_log::Query { orgs: vec![org_id], since_ms: Some(since), limit: 500, ..Default::default() };
    let (mut replayed, mut skipped, mut read) = (0usize, 0usize, 0usize);
    let mut flips: Vec<Value> = Vec::new();
    while read < limit {
        let page = match flowr_ce::decision_log::recall(&q).await {
            Ok((rows, _)) => rows,
            Err(e) => return refuse(StatusCode::SERVICE_UNAVAILABLE, format!("the decision log can't be read: {e}")),
        };
        if page.is_empty() {
            break;
        }
        for r in &page {
            read += 1;
            let action = s(r, "action");
            if EVENTS.iter().any(|e| action.starts_with(e)) {
                skipped += 1;
                continue;
            }
            let Some(req) = request_of(r) else { skipped += 1; continue };
            replayed += 1;
            let (before, after) = (now_hook.check(&req).await, cand_hook.check(&req).await);
            if word(&before) != word(&after) {
                let (ids, reason) = match &after {
                    PolicyDecision::Deny { policy_ids, reason } | PolicyDecision::Ask { policy_ids, reason, .. } => (policy_ids.clone(), reason.clone()),
                    PolicyDecision::Allow => match &before {
                        PolicyDecision::Deny { policy_ids, .. } | PolicyDecision::Ask { policy_ids, .. } => (policy_ids.clone(), String::new()),
                        PolicyDecision::Allow => (vec![], String::new()),
                    },
                };
                if flips.len() < 500 {
                    flips.push(json!({
                        "ts_ms": r.get("ts_ms"), "id": s(r, "id"), "principal": s(r, "principal_id"), "on_behalf_of": s(r, "on_behalf_of"),
                        "agent": s(r, "agent"), "action": action, "resource": format!("{}::{}", s(r, "resource_type"), s(r, "resource_id")),
                        "project": s(r, "project"), "logged": s(r, "decision"), "before": word(&before), "after": word(&after),
                        "policy_ids": ids, "reason": reason,
                    }));
                }
            }
        }
        let last = page.last().and_then(|r| r["ts_ms"].as_u64());
        if page.len() < q.limit || last.is_none() {
            break;
        }
        q.before_ms = last;
    }
    let flipped = flips.len();
    Json(json!({
        "org": org, "days": days, "read": read, "replayed": replayed, "skipped": skipped,
        "flipped": flipped, "unchanged": replayed - flipped.min(replayed), "flips": flips,
        "note": "Each decision is rebuilt from the log and decided by today's policies and by the changed ones; attributes the log doesn't keep are absent from both.",
    })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_logged_decision_rebuilds_its_request() {
        let r = json!({ "principal_kind": "agent", "principal_id": "agent:a1:s1", "roles": ["Operator"], "on_behalf_of": "u1",
            "agent": "a1", "agent_owner": "u1", "action": "mcp:call", "resource_type": "Tool", "resource_id": "delete_flow", "project": "acme-web" });
        let req = request_of(&r).expect("rebuilt");
        assert_eq!(req.principal.kind(), "agent");
        assert_eq!(req.principal.on_behalf_of(), Some("u1"));
        assert_eq!(req.resource.attributes["namespace"], json!("acme-web"));
        assert_eq!(req.resource.attributes["name"], json!("delete_flow"));
        assert_eq!(req.attributes["template"]["agent"], json!("a1"));
        assert!(request_of(&json!({ "principal_kind": "x", "action": "mcp:call", "resource_type": "Tool" })).is_none());
    }
}
