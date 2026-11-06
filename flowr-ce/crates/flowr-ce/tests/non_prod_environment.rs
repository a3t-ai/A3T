//! AC-7: a gateway serving dev or QA lets nothing out. Own test binary: it sets
//! `A3T_ENVIRONMENT` for the whole process.

use flowr_ce::gateway::outbound_blocked;

#[test]
fn outside_production_only_the_model_is_allowed_out() {
    let alice = flowr_policy::Principal::human("alice", vec![]);
    for env in ["dev", "qa"] {
        std::env::set_var("A3T_ENVIRONMENT", env);
        for t in ["call:http", "call:mcp"] {
            let reason = outbound_blocked(&alice, t, "http://x").unwrap_or_else(|| panic!("{t} allowed in {env}"));
            assert!(reason.contains(env) && reason.contains("http://x"), "{reason}");
        }
        assert!(outbound_blocked(&alice, "call:llm", "llm:x").is_none());
    }
    for env in ["prod", "production", ""] {
        std::env::set_var("A3T_ENVIRONMENT", env);
        assert!(outbound_blocked(&alice, "call:http", "http://x").is_none(), "{env}");
    }
    std::env::remove_var("A3T_ENVIRONMENT");
    assert!(outbound_blocked(&alice, "call:http", "http://x").is_none());
}
