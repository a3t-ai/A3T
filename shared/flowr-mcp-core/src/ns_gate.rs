//! The namespace gate for MCP tools: a caller sees and changes only the projects
//! it may (`flowr_policy::namespace`). Single-item tools are refused before they
//! run; list tools filter what they return. With the default guard (CE) nothing
//! is refused.

use serde_json::{json, Value};

use flowr_policy::namespace::{acting_user, is_superadmin, namespace_guard, ItemKind, NsAccess};
use flowr_policy::Principal;
use flowr_storage::StorageBackend;
use flowr_types::ExecutionState;

use crate::protocol::CallToolResult;

fn s<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(|v| v.as_str()).filter(|v| !v.is_empty())
}

/// `document.namespace` of a flow definition given as YAML or JSON.
fn definition_namespace(args: &Value) -> Option<String> {
    // Parsed, not scanned: `document: { namespace: x }` is the same document.
    if let Some(y) = s(args, "yaml") {
        let doc: Value = serde_yaml::from_str(y).ok()?;
        return doc.get("document").and_then(|d| d.get("namespace")).and_then(|v| v.as_str()).map(String::from);
    }
    args.get("json").and_then(|j| j.get("document")).and_then(|d| d.get("namespace")).and_then(|v| v.as_str()).map(String::from)
}

fn trigger_namespace(t: &Value) -> String {
    let ns = t.get("namespace").and_then(|v| v.as_str()).unwrap_or("default");
    let name = t.get("name").and_then(|v| v.as_str()).unwrap_or_default();
    namespace_guard().effective_namespace(ItemKind::Trigger, ns, name, "", "")
}

/// What it takes to run `tool` with `args`: the namespace and the access needed.
/// `None` = not namespace-scoped (or a list tool, filtered afterwards).
async fn requirement(storage: &dyn StorageBackend, tool: &str, args: &Value) -> Option<(String, NsAccess)> {
    let g = namespace_guard();
    let ns = || s(args, "namespace").unwrap_or("default").to_string();
    let flow = |level| {
        let ns = ns();
        let name = s(args, "name").unwrap_or_default();
        Some((g.effective_namespace(ItemKind::Flow, &ns, name, "", ""), level))
    };
    match tool {
        "list_flows" | "list_resources" | "list_mcp_tools" => Some((ns(), NsAccess::Read)),
        "get_flow" => flow(NsAccess::Read),
        "execute_flow" => flow(NsAccess::Run),
        "update_flow" | "delete_flow" => flow(NsAccess::Write),
        "create_flow" => Some((definition_namespace(args).unwrap_or_else(|| "default".into()), NsAccess::Write)),
        "get_resource" | "delete_resource" => {
            let ns = ns();
            let name = s(args, "name").unwrap_or_default();
            let type_ref = storage.resources().get(&ns, name).await.map(|r| r.type_ref).unwrap_or_default();
            let level = if tool == "get_resource" { NsAccess::Read } else { NsAccess::Write };
            Some((g.effective_namespace(ItemKind::Resource, &ns, name, &type_ref, ""), level))
        }
        "create_resource" => Some((ns(), NsAccess::Write)),
        "create_trigger" => Some((trigger_namespace(args), NsAccess::Write)),
        "get_trigger" | "update_trigger" | "delete_trigger" | "enable_trigger" | "pause_trigger"
        | "attach_trigger" | "detach_trigger" => {
            let id = s(args, "id").or_else(|| s(args, "trigger_id")).unwrap_or_default();
            let t = storage.triggers().get(id).await.ok().flatten().unwrap_or(json!({}));
            let level = if tool == "get_trigger" { NsAccess::Read } else { NsAccess::Write };
            Some((trigger_namespace(&t), level))
        }
        _ => None,
    }
}

/// Refuse a namespace-scoped tool the caller may not use. A namespace the caller
/// can't see at all answers as if it didn't exist.
pub async fn check(storage: &dyn StorageBackend, principal: &Principal, tool: &str, args: &Value) -> Option<CallToolResult> {
    let g = namespace_guard();
    if !g.enforcing() {
        return None;
    }
    if tool == "get_execution_status" {
        let id = s(args, "instance_id").and_then(|i| uuid::Uuid::parse_str(i).ok())?;
        let run = storage.executions().get(id).await.ok().flatten()?;
        return (!can_see_run(principal, &run).await)
            .then(|| CallToolResult::text(format!("Execution `{id}` not found or still pending.")));
    }
    let (ns, needed) = requirement(storage, tool, args).await?;
    if needed == NsAccess::Read && flowr_policy::namespace::platform_hidden(principal, &ns) {
        refused(principal, &ns, tool, needed, NsAccess::None);
        return Some(CallToolResult::error(format!("Not found: no project '{ns}' that you can see.")));
    }
    let has = g.access(principal, &ns).await;
    if has >= needed {
        return None;
    }
    refused(principal, &ns, tool, needed, has);
    Some(if has == NsAccess::None {
        CallToolResult::error(format!("Not found: no project '{ns}' that you can see."))
    } else {
        CallToolResult::error(format!("Forbidden: this needs {} access to project '{ns}'; you have {}.",
            level(needed), level(has)))
    })
}

/// AUD-3: a refusal by project isolation goes in the decision log.
pub fn refused(principal: &Principal, ns: &str, what: &str, needed: NsAccess, has: NsAccess) {
    flowr_policy::audit::event(principal, "project:access", "Project", ns, Some(ns), None,
        Some(&format!("{what} needs {} access; the caller has {} access", level(needed), level(has))), None);
}

fn level(a: NsAccess) -> &'static str {
    match a {
        NsAccess::None => "no",
        NsAccess::Read => "read",
        NsAccess::Run => "run",
        NsAccess::Write => "write",
    }
}

/// Whether `principal` may see `item` (a flow, resource or trigger in `ns`).
pub async fn can_read(principal: &Principal, kind: ItemKind, ns: &str, name: &str, type_ref: &str, created_by: &str) -> bool {
    let g = namespace_guard();
    if !g.enforcing() {
        return true;
    }
    let ns = g.effective_namespace(kind, ns, name, type_ref, created_by);
    !flowr_policy::namespace::platform_hidden(principal, &ns) && g.access(principal, &ns).await >= NsAccess::Read
}

/// Whether `principal` may see a run. A run of a project's flow: whoever can read
/// the project. A run of a platform flow (a chat, a CRUD call): only who ran it,
/// and superadmin — they carry other people's conversations and data.
pub async fn can_see_run(principal: &Principal, run: &ExecutionState) -> bool {
    can_see(principal, &run.namespace, &run.name, run.principal.as_ref()).await
}

/// Whether `principal` may see a run of `namespace/flow` started by `started_by`.
pub async fn can_see(principal: &Principal, namespace: &str, flow: &str, started_by: Option<&Principal>) -> bool {
    let g = namespace_guard();
    if !g.enforcing() || matches!(principal, Principal::Service { .. }) {
        return true;
    }
    let ns = g.effective_namespace(ItemKind::Flow, namespace, flow, "", "");
    if g.is_platform(&ns) {
        if is_superadmin(principal) {
            return true;
        }
        let me = acting_user(principal);
        let ran = started_by.and_then(acting_user);
        return me.is_some() && me == ran;
    }
    g.access(principal, &ns).await >= NsAccess::Read
}

#[cfg(test)]
mod definition_namespace_tests {
    use super::*;

    #[test]
    fn the_namespace_is_parsed_whatever_the_yaml_style() {
        let block = json!({ "yaml": "document:\n  dsl: 1.0.3\n  namespace: system\n  name: x\n  version: 1.0.0\ndo: []\n" });
        let flow_style = json!({ "yaml": "document: { dsl: 1.0.3, namespace: system, name: x, version: 1.0.0 }\ndo: []\n" });
        let quoted = json!({ "yaml": "document:\n  namespace: \"system\"\n" });
        let as_json = json!({ "json": { "document": { "namespace": "system" } } });
        for a in [block, flow_style, quoted, as_json] {
            assert_eq!(definition_namespace(&a).as_deref(), Some("system"), "{a}");
        }
        // A `namespace:` outside the document doesn't count.
        let elsewhere = json!({ "yaml": "document: { name: x }\ndo:\n  - a:\n      with: { namespace: system }\n" });
        assert_eq!(definition_namespace(&elsewhere), None);
    }
}
