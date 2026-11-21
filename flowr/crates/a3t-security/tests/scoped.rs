//! Project policies: a policy saved in a project governs the calls that touch
//! that project only; decisions name policies by `namespace/name` (or a baseline
//! rule's `@id`); a project policy that does not compile closes that project only.

use a3t_security::{CedarHook, NamedPolicy};
use flowr_policy::{actions, ExecutionRequest, PolicyDecision, PolicyHook, Principal, Resource};

fn start_flow_in(ns: &str) -> ExecutionRequest {
    ExecutionRequest {
        principal: Principal::Human { session: None, memory_branch: None, impersonated_by: None, user_id: "op".into(), roles: vec!["Operator".into()] },
        action: actions::flow_start(),
        resource: Resource::new("Flow", "flow_x").with_attr("namespace", ns),
        context: Default::default(),
        attributes: Default::default(),
        params: serde_json::json!({}),
    }
}

const NO_FLOWS: &str = "forbid(principal, action == Action::\"flow:start\", resource);";

#[tokio::test]
async fn a_project_policy_applies_in_its_project_only() {
    let hook = CedarHook::with_named_policies(&[NamedPolicy::new("acme-web", "no-flows", NO_FLOWS)], None, &[]).unwrap();
    match hook.check(&start_flow_in("acme-web")).await {
        PolicyDecision::Deny { policy_ids, reason } => {
            assert_eq!(policy_ids, vec!["acme-web/no-flows".to_string()]);
            assert!(reason.contains("acme-web/no-flows"), "{reason}");
        }
        d => panic!("expected deny, got {d:?}"),
    }
    assert!(matches!(hook.check(&start_flow_in("globex-x")).await, PolicyDecision::Allow));
}

#[tokio::test]
async fn a_platform_policy_applies_everywhere() {
    let hook = CedarHook::with_named_policies(&[NamedPolicy::new("a3t", "no-flows", NO_FLOWS)], None, &[]).unwrap();
    assert!(matches!(hook.check(&start_flow_in("acme-web")).await, PolicyDecision::Deny { .. }));
    assert!(matches!(hook.check(&start_flow_in("globex-x")).await, PolicyDecision::Deny { .. }));
}

#[tokio::test]
async fn a_broken_project_policy_closes_that_project_only() {
    let hook = CedarHook::with_named_policies(&[NamedPolicy::new("acme-web", "bad", "forbid(principal, action, resource) when { nonsense };")], None, &[]).unwrap();
    assert!(matches!(hook.check(&start_flow_in("acme-web")).await, PolicyDecision::Deny { .. }));
    assert!(matches!(hook.check(&start_flow_in("globex-x")).await, PolicyDecision::Allow));
    let e = CedarHook::validate(&[NamedPolicy::new("acme-web", "bad", "forbid(principal, action, resource) when { nonsense };")], None, &[]).unwrap_err();
    assert!(e.contains("acme-web"), "{e}");
}

#[tokio::test]
async fn baseline_refusals_name_the_rule_and_the_access_needed() {
    let hook = CedarHook::baseline().unwrap();
    let req = ExecutionRequest {
        principal: Principal::Human { session: None, memory_branch: None, impersonated_by: None, user_id: "v".into(), roles: vec!["Viewer".into()] },
        action: actions::tool_invoke(),
        resource: Resource::new("Tool", "delete_flow").with_attr("destructive", true),
        context: Default::default(),
        attributes: Default::default(),
        params: serde_json::json!({}),
    };
    match hook.check(&req).await {
        PolicyDecision::Deny { policy_ids, reason } => {
            assert_eq!(policy_ids, vec!["baseline/destructive-tools".to_string()]);
            assert!(reason.contains("Admin role"), "{reason}");
        }
        d => panic!("expected deny, got {d:?}"),
    }
}

#[tokio::test]
async fn an_allow_names_the_permit_in_a_dry_run() {
    let hook = CedarHook::baseline().unwrap();
    let (d, ids) = hook.check_explained(&start_flow_in("acme-web")).await;
    assert!(matches!(d, PolicyDecision::Allow));
    assert_eq!(ids, vec!["baseline/humans-run-steps".to_string()]);
}
