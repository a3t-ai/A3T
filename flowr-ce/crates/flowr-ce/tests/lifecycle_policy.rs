//! AC-14: a policy deny on a lifecycle action blocks it before anything happens. Own test
//! binary: it installs a process-wide hook.

use std::sync::Arc;

use flowr_policy::{ExecutionRequest as PolicyRequest, PolicyDecision, PolicyHook};
use flowr_storage::StorageBackend;
use flowr_types::Resource;
use serde_json::{json, Value};

struct DenyLifecycle;

#[async_trait::async_trait]
impl PolicyHook for DenyLifecycle {
    async fn check(&self, req: &PolicyRequest) -> PolicyDecision {
        let action = format!("{:?}", req.action);
        if action.contains("memory:") || action.contains("deploy:prod") {
            PolicyDecision::Deny { reason: format!("denied {action}"), policy_ids: vec!["test".into()] }
        } else {
            PolicyDecision::Allow
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deny_blocks_each_lifecycle_action() {
    flowr_policy::set_policy_hook(Arc::new(DenyLifecycle)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let storage: Arc<dyn StorageBackend> = flowr_ce::init_storage(dir.path().to_str().unwrap()).await.unwrap();
    let db_path = dir.path().join("platform.lance");
    storage.resources().create(Resource::new("a3t-platform-db", "lancedb", json!({ "db_path": db_path.to_str().unwrap() })).with_namespace("a3t")).await.unwrap();
    let db = flowr_ce::platform_db::open_platform_db(storage.as_ref(), Some(flowr_ce::handlers::call_lancedb::PLATFORM_DB_RESOURCE), flowr_ce::handlers::call_lancedb::DEFAULT_PLATFORM_DB_PATH).await;
    db.insert("agent_configs", &[json!({ "id": "bot", "org_id": "o", "name": "Bot", "template_id": "t", "template_version": "1", "namespace": "acme" })]).await.unwrap();
    flowr_ce::agent_space::set_space(storage.as_ref(), "bot", "o", "acme--bot", "me").await.unwrap();

    let app = flowr_ce::agent_space::router(storage.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();

    let calls: Vec<(reqwest::Method, &str, Value)> = vec![
        (reqwest::Method::POST, "/agents/bot/branches", json!({ "name": "b" })),
        (reqwest::Method::POST, "/agents/bot/branches/b/merge", json!({ "main_offset": 1 })),
        (reqwest::Method::POST, "/agents/bot/branches/b/rebase", json!({ "main_offset": 1 })),
        (reqwest::Method::POST, "/agents/bot/merges/m/revert", json!({})),
        (reqwest::Method::POST, "/agents/bot/branches/b/promote", json!({ "new_agent": { "id": "n" } })),
        (reqwest::Method::POST, "/agents/bot/fork", json!({ "new_agent": { "id": "n" } })),
        (reqwest::Method::PUT, "/agents/bot/layers", json!({ "layers": [] })),
    ];
    for (method, path, body) in calls {
        let r = client.request(method, format!("{url}{path}")).json(&body).send().await.unwrap();
        let status = r.status().as_u16();
        let text = r.text().await.unwrap();
        assert_eq!(status, 403, "{path}: {text}");
        assert!(text.contains("policy_denied"), "{path}: {text}");
    }
    // Pinning prod is checked before the version is looked at; dev isn't a prod action.
    let r = client.put(format!("{url}/agents/bot/deployments/prod")).json(&json!({ "version": "1" })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 403);
    let r = client.put(format!("{url}/agents/bot/deployments/dev")).json(&json!({ "version": "1" })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 404, "no such version, but not refused by policy");

    // AC-15: the gateway asks the same policy, as the caller, and says why.
    let call = flowr_ce::gateway::Call {
        task_type: "call:http".into(),
        target: "https://x".into(),
        caller_run: "r".into(),
        caller_step: None,
        namespace: "t".into(),
        flow: "f".into(),
        principal: serde_json::to_value(flowr_policy::Principal::human("alice", vec![])).unwrap(),
    };
    let resource = flowr_policy::Resource::new("Tool", "x");
    let denied = flowr_ce::gateway::policy_decision(flowr_policy::Action::new("memory:branch").unwrap(), resource.clone(), &call).await;
    assert!(!denied.allow && denied.reason.contains("denied"), "{denied:?}");
    let ok = flowr_ce::gateway::policy_decision(flowr_policy::Action::new("mcp:call").unwrap(), resource.clone(), &call).await;
    assert!(ok.allow, "{ok:?}");
    let mut unknown = call.clone();
    unknown.principal = Value::Null;
    assert!(!flowr_ce::gateway::policy_decision(flowr_policy::Action::new("mcp:call").unwrap(), resource, &unknown).await.allow, "no identity, no call");
}
