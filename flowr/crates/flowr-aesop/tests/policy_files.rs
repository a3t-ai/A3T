//! Org policy and role files compile to Cedar that the engine enforces, and
//! they can only narrow access.

use flowr_aesop::policy_files::{compile_agent_policy, compile_policy, compile_role, HiddenApp, Placement};
use flowr_aesop::CedarHook;
use flowr_policy::{actions, ExecutionRequest, PolicyDecision, PolicyHook, Principal, Resource};

fn human(roles: &[&str]) -> Principal {
    Principal::Human { memory_branch: None, impersonated_by: None, user_id: "u1".into(), roles: roles.iter().map(|r| r.to_string()).collect() }
}

fn llm_call(p: Principal, model: &str) -> ExecutionRequest {
    ExecutionRequest { principal: p, action: actions::llm_call(), resource: Resource::new("Llm", model).with_attr("name", model),
        context: Default::default(), attributes: Default::default(), params: serde_json::json!({}) }
}

fn tool_call(p: Principal, tool: &str, template: serde_json::Value) -> ExecutionRequest {
    ExecutionRequest { principal: p, action: actions::tool_invoke(), resource: Resource::tool(tool),
        context: Default::default(), attributes: serde_json::json!({ "template": template }).as_object().unwrap().clone(),
        params: serde_json::json!({}) }
}

fn hook(cedar: &[String]) -> CedarHook {
    CedarHook::with_registry_policies(cedar, None, &[]).expect("compiled policy loads and validates")
}

fn denied(d: &PolicyDecision) -> Option<String> {
    match d { PolicyDecision::Deny { reason, .. } => Some(reason.clone()), _ => None }
}

#[tokio::test]
async fn a_plain_rule_forbids_only_what_it_says() {
    let c = compile_policy("acme", r#"
kind: a3t/policy
name: approved-models
applies_to: { org: true }
rules:
  - who: { roles: [Operator] }
    cannot: [models.use]
    on: { models: { not_in: [acme/claude-sonnet] } }
    reason: Only the org's approved models are allowed.
"#).unwrap();
    assert_eq!(c.placement, Placement::Org);
    let h = hook(&[c.cedar]);
    let why = denied(&h.check(&llm_call(human(&["Operator", "Viewer"]), "openai/gpt")).await).expect("other model refused");
    assert!(why.contains("approved models"), "{why}");
    assert!(denied(&h.check(&llm_call(human(&["Operator"]), "acme/claude-sonnet")).await).is_none(), "approved model allowed");
    assert!(denied(&h.check(&llm_call(human(&["Admin"]), "openai/gpt")).await).is_none(), "the rule names Operators only");
}

#[tokio::test]
async fn a_role_is_its_base_minus_what_it_removes() {
    let c = compile_role("acme", r#"
kind: a3t/role
name: support-operator
extends: Admin
remove: [tools.delete_*, apps.security]
"#).unwrap();
    assert_eq!(c.hidden_apps, vec![HiddenApp { app: "security".into(), roles: vec!["acme/support-operator".into()] }]);
    let h = hook(&[c.cedar]);
    let support = human(&["Admin", "acme/support-operator"]);
    assert!(denied(&h.check(&tool_call(support.clone(), "delete_flow", serde_json::json!({}))).await).is_some(), "removed tool refused");
    assert!(denied(&h.check(&tool_call(support, "list_flows", serde_json::json!({}))).await).is_none(), "the rest of Admin stays");
    assert!(denied(&h.check(&tool_call(human(&["Admin"]), "delete_flow", serde_json::json!({}))).await).is_none(), "plain Admin unaffected");
}

#[tokio::test]
async fn applies_to_tags_and_references_scope_a_rule() {
    let c = compile_policy("acme", r#"
kind: a3t/policy
name: no-deletes-on-pii
applies_to: { tags: [pii] }
rules:
  - cannot: [flows.delete]
    reason: Flows that touch personal data are never deleted by a run.
"#).unwrap();
    let h = hook(&[c.cedar]);
    let admin = human(&["Admin"]);
    assert!(denied(&h.check(&tool_call(admin.clone(), "delete_flow", serde_json::json!({"tags": ["pii"]}))).await).is_some(), "tagged flow bound");
    assert!(denied(&h.check(&tool_call(admin.clone(), "delete_flow", serde_json::json!({"tags": ["billing"]}))).await).is_none(), "other tags free");
    assert!(denied(&h.check(&tool_call(admin.clone(), "delete_flow", serde_json::json!({"policyRef": ["acme/no-deletes-on-pii"]}))).await).is_some(), "a reference attaches it too");
    assert!(denied(&h.check(&tool_call(admin, "delete_flow", serde_json::json!({"flow": "acme-crm/refunds"}))).await).is_none(), "not named, not tagged");
}

#[tokio::test]
async fn applies_to_flows_by_name_without_touching_the_flow() {
    let c = compile_policy("acme", r#"
kind: a3t/policy
name: refunds-no-external-tools
applies_to: { flows: [acme-crm/refunds] }
rules:
  - cannot: [tools.external]
    on: { tools: ["github*"] }
    reason: The refunds flow may not call GitHub.
"#).unwrap();
    let h = hook(&[c.cedar]);
    let p = human(&["Admin"]);
    assert!(denied(&h.check(&tool_call(p.clone(), "github_create_issue", serde_json::json!({"flow": "acme-crm/refunds"}))).await).is_some());
    assert!(denied(&h.check(&tool_call(p.clone(), "slack_post", serde_json::json!({"flow": "acme-crm/refunds"}))).await).is_none(), "other external tools allowed");
    assert!(denied(&h.check(&tool_call(p.clone(), "list_flows", serde_json::json!({"flow": "acme-crm/refunds"}))).await).is_none(), "built-in tools are not external");
    assert!(denied(&h.check(&tool_call(p, "github_create_issue", serde_json::json!({"flow": "acme-crm/other"}))).await).is_none(), "other flows free");
}

#[test]
fn org_policy_can_never_widen_access() {
    let permit = compile_policy("acme", r#"
kind: a3t/policy
name: sneaky
cedar: |
  permit(principal, action, resource);
"#);
    assert!(permit.unwrap_err().contains("only `forbid`"));
}

#[test]
fn mistakes_are_refused_with_a_reason() {
    let bad = |y: &str| compile_policy("acme", y).unwrap_err();
    assert!(bad("kind: a3t/policy\nname: x\nrules:\n  - cannot: [flows.fly]\n    reason: r\n").contains("not in the permission catalogue"));
    assert!(bad("kind: a3t/policy\nname: x\nrules:\n  - cannot: [flows.run]\n    reason: ''\n").contains("give a reason"));
    assert!(bad("kind: a3t/policy\nname: Bad_Name\nrules:\n  - cannot: [flows.run]\n    reason: r\n").contains("lowercase"));
    assert!(bad("kind: a3t/policy\nname: x\nrules:\n  - cannot: [flows.run]\n    reason: r\n    whenever: now\n").contains("unknown field"));
    assert!(compile_role("acme", "kind: a3t/role\nname: x\nextends: SuperAdmin\n").unwrap_err().contains("extends"));
}

#[test]
fn expert_statements_are_named_after_their_file() {
    let c = compile_policy("acme", r#"
kind: a3t/policy
name: expert
cedar: |
  forbid(principal, action == Action::"mcp:call", resource)
  when { resource has name && resource.name == "delete_flow" };
"#).unwrap();
    assert!(c.cedar.contains("@id(\"acme/expert/cedar.1\")"), "{}", c.cedar);
    hook(&[c.cedar]);
}

#[test]
fn expert_cedar_that_does_not_fit_the_schema_is_refused_at_save() {
    let e = compile_policy("acme", "kind: a3t/policy\nname: x\ncedar: |\n  forbid(principal, action, resource) when { resource.colour == \"red\" };\n").unwrap_err();
    assert!(e.contains("A3T schema"), "{e}");
}

#[tokio::test]
async fn an_agents_own_policy_binds_that_agent_only() {
    let yaml = "kind: a3t/policy\nname: profile\nrules:\n  - cannot: [tools.external]\n    reason: This agent uses only A3T's own tools.\n";
    let c = compile_agent_policy("acme-crm", "agt-1", yaml).unwrap();
    let h = hook(&[c.cedar]);
    let as_agent = |id: &str| tool_call(human(&["Admin"]), "github_create_issue", serde_json::json!({"agent": id}));
    assert!(denied(&h.check(&as_agent("agt-1")).await).is_some(), "the agent is bound");
    assert!(denied(&h.check(&as_agent("agt-2")).await).is_none(), "other agents are not");
    // A copy under a new id carries the same file, scoped to itself.
    let copy = compile_agent_policy("other-org", "agt-9", yaml).unwrap();
    assert!(denied(&hook(&[copy.cedar]).check(&as_agent("agt-9")).await).is_some());
    assert!(compile_agent_policy("acme-crm", "agt-1", "kind: a3t/policy\nname: x\napplies_to: { org: true }\nrules:\n  - cannot: [flows.run]\n    reason: r\n").is_err());
}
