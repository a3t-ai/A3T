//! Org policy and role files compile to Cedar that the engine enforces, and
//! they can only narrow access.

use a3t_security::policy_files::{compile_agent_policy, compile_org_set, compile_policy, compile_role, HiddenApp, OrgFile, Placement};
use a3t_security::CedarHook;
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

/// A chat turn: the agent (acting for a user, with its session id as its id)
/// calls its model; the runtime stamps `template.agent`.
#[tokio::test]
async fn an_agents_model_rule_holds_on_its_chat_turns() {
    let yaml = "kind: a3t/policy\nname: profile\nrules:\n  - cannot: [models.use]\n    on: { models: { not_in: [a3t/approved] } }\n    reason: It runs only on the approved model.\n";
    let h = hook(&[compile_agent_policy("acme-crm", "agt-1", yaml).unwrap().cedar]);
    let turn = |agent: &str, model: &str| {
        let mut r = llm_call(Principal::Agent {
            memory_branch: None, agent_id: flowr_policy::AgentId(format!("agent:{agent}:s1")),
            on_behalf_of: Some("u1".into()), flow_id: None, roles: vec!["Operator".into()], owner: None,
        }, model);
        r.attributes = serde_json::json!({ "template": { "agent": agent } }).as_object().unwrap().clone();
        r
    };
    assert!(denied(&h.check(&turn("agt-1", "a3t/other")).await).is_some(), "another model is refused");
    assert!(denied(&h.check(&turn("agt-1", "a3t/approved")).await).is_none(), "the approved model runs");
    assert!(denied(&h.check(&turn("agt-2", "a3t/other")).await).is_none(), "other agents are not bound");
}

fn file(path: &str, content: &str) -> OrgFile {
    OrgFile { path: path.into(), content: content.into() }
}

const MODELS: &str = "kind: a3t/policy\nname: approved-models\napplies_to: { org: true }\nrules:\n  - cannot: [models.use]\n    on: { models: { not_in: [acme/claude] } }\n    reason: Only approved models.\n";
const ROLE: &str = "kind: a3t/role\nname: support-lead\nextends: Operator\nremove: [memory.change]\n";

#[tokio::test]
async fn an_orgs_files_compile_into_its_own_projects_only() {
    let projects = vec!["acme-crm".to_string(), "acme-support".to_string()];
    let set = compile_org_set("acme", &projects, &[
        file("policies/approved-models.yaml", MODELS),
        file("roles/support-lead.yaml", ROLE),
        file("access/acme-support.yaml", "kind: a3t/access\nproject: acme-support\ngroups: { support-leads: support-lead, support: Operator }\nusers: { jane@acme.com: Viewer }\n"),
        file("README.md", "not ours"),
        file("policies/nested/x.yaml", "ignored: not directly in policies/"),
    ]).expect("a good set compiles");
    let placed: Vec<(&str, &str)> = set.policies.iter().map(|(p, n, _)| (p.as_str(), n.as_str())).collect();
    assert!(placed.contains(&("acme-crm", "org.acme.policy.approved-models")));
    assert!(placed.contains(&("acme-support", "org.acme.policy.approved-models")));
    assert!(placed.contains(&("acme-support", "org.acme.role.support-lead")));
    assert!(placed.iter().all(|(p, _)| projects.iter().any(|q| q == p)), "nothing outside the org");
    assert_eq!(set.access.len(), 1);
    assert_eq!(set.files.len(), 3);
    // What each project carries is enforced there.
    let h = hook(&[set.policies[0].2.clone()]);
    assert!(denied(&h.check(&llm_call(human(&["Admin"]), "openai/gpt")).await).is_some());
}

#[test]
fn a_set_with_any_bad_file_is_refused_whole_and_names_each_file() {
    let projects = vec!["acme-crm".to_string()];
    let errs = compile_org_set("acme", &projects, &[
        file("policies/approved-models.yaml", MODELS),
        file("policies/elsewhere.yaml", "kind: a3t/policy\nname: elsewhere\napplies_to: { projects: [globex-prod] }\nrules:\n  - cannot: [flows.delete]\n    reason: r\n"),
        file("policies/broken.yaml", "kind: a3t/policy\nname: broken\nrules:\n  - cannot: [no.such]\n    reason: r\n"),
        file("access/acme-crm.yaml", "kind: a3t/access\nproject: acme-crm\ngroups: { ops: Superuser }\n"),
        file("access/globex.yaml", "kind: a3t/access\nproject: globex-prod\ngroups: { ops: Viewer }\n"),
        file("roles/dup.yaml", ROLE),
        file("roles/dup2.yaml", ROLE),
    ]).unwrap_err();
    let all = errs.join("\n");
    for needle in ["policies/elsewhere.yaml: applies_to names projects that are not acme's: globex-prod",
                   "policies/broken.yaml:", "access/acme-crm.yaml: ops → \"Superuser\"",
                   "access/globex.yaml: project \"globex-prod\" is not acme's", "roles/dup2.yaml: another role file"] {
        assert!(all.contains(needle), "expected {needle:?} in:\n{all}");
    }
}

/// A rule binds a role that exists: the org's own (with or without the org
/// prefix), a baseline one, or a platform one. A misnamed role would bind nobody.
#[test]
fn a_rule_names_only_roles_that_exist() {
    let projects = vec!["acme-crm".to_string()];
    let rule = |roles: &str| file("policies/leads.yaml", &format!("kind: a3t/policy\nname: leads\napplies_to: {{ org: true }}\nrules:\n  - who: {{ roles: [{roles}] }}\n    cannot: [flows.delete]\n    reason: r\n"));
    for ok in ["support-lead", "acme/support-lead", "Operator", "SecurityAdmin"] {
        compile_org_set("acme", &projects, &[rule(ok), file("roles/lead.yaml", ROLE)]).unwrap_or_else(|e| panic!("{ok}: {e:?}"));
    }
    for bad in ["support-leed", "globex/support-lead"] {
        let errs = compile_org_set("acme", &projects, &[rule(bad), file("roles/lead.yaml", ROLE)]).unwrap_err().join("\n");
        assert!(errs.contains(&format!("policies/leads.yaml: rule 1: who.roles names {bad:?}")), "{bad}: {errs}");
    }
    // The role file gone: the rule that binds it no longer compiles.
    assert!(compile_org_set("acme", &projects, &[rule("support-lead")]).is_err());
}

/// The test policy repo in this repo (`tests/policy-repo/acme`, what vc-dev's
/// org `acme` syncs from) compiles as FlowR will compile it.
#[test]
fn the_test_policy_repo_compiles() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/policy-repo/acme");
    let mut files = Vec::new();
    for dir in ["policies", "roles", "access"] {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = e.unwrap().path();
            files.push(file(&format!("{dir}/{}", p.file_name().unwrap().to_str().unwrap()), &std::fs::read_to_string(&p).unwrap()));
        }
    }
    let set = compile_org_set("acme", &["acme-p1".to_string(), "acme-sam1".to_string()], &files).expect("the test repo compiles");
    assert_eq!(set.files.len(), 4);
    let p1: Vec<&str> = set.policies.iter().filter(|(p, _, _)| p == "acme-p1").map(|(_, n, _)| n.as_str()).collect();
    assert!(p1.contains(&"org.acme.policy.no-flow-deletes-p1") && p1.contains(&"org.acme.policy.approved-models"), "{p1:?}");
    assert!(!set.policies.iter().any(|(p, n, _)| p == "acme-sam1" && n.contains("no-flow-deletes")), "p1's rule stays in p1");
}
