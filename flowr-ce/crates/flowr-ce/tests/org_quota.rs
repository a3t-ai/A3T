//! AC-18: a new run is refused at admission, with a plain message, once an org's active runs
//! across its projects reach its quota. Other orgs and the platform aren't affected.

use std::sync::Arc;

use flowr_storage::engine::{Durability, NewRun, NewTask};
use flowr_storage::StorageBackend;
use flowr_types::{ExecutionRequest, ExecutionStatus, Resource, WorkflowRecord};
use serde_json::{json, Value};

const SUPER: &[&str] = &["SuperAdmin", "SecurityAdmin", "Admin", "Operator", "Viewer"];

fn who(user: &str) -> flowr_policy::Principal {
    flowr_policy::Principal::human(user, SUPER.iter().map(|r| r.to_string()).collect())
}

async fn flow(storage: &Arc<dyn StorageBackend>, ns: &str, name: &str, input: Value) {
    let req = ExecutionRequest::new(ns, name).with_input(input).with_principal(who("root"));
    let st = flowr_ce::execute_workflow(storage.clone(), &reqwest::Client::new(), req).await.unwrap();
    assert_eq!(st.status, ExecutionStatus::Completed, "{name}: {:?}", st.error);
}

fn pending(ns: &str) -> NewRun {
    NewRun {
        run_id: uuid::Uuid::now_v7(),
        namespace: ns.to_string(),
        flow: "held".to_string(),
        version: String::new(),
        def_hash: String::new(),
        parent_run_id: None,
        durability: Durability::Durable,
        debug: false,
        breakpoints: None,
        principal: Value::Null,
        trigger: "test".into(),
        input: json!({}),
        deadline_at: None,
        dedupe_key: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn runs_over_an_orgs_quota_are_refused_at_admission() {
    let dir = tempfile::tempdir().unwrap();
    let storage = flowr_ce::init_storage(dir.path().to_str().unwrap()).await.unwrap();
    let db_path = dir.path().join("platform.lance");
    storage.resources().create(Resource::new("a3t-platform-db", "lancedb", json!({ "db_path": db_path.to_str().unwrap() })).with_namespace("a3t")).await.unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../a3t/backend/flows");
    for f in ["system/a3t_base.yaml", "system/a3t_error_handler.yaml", "system/a3t_projects_install.yaml", "a3t/a3t_profile.yaml", "a3t/a3t_orgs.yaml", "a3t/a3t_projects.yaml"] {
        storage.workflows().register(WorkflowRecord::from_yaml(&std::fs::read_to_string(root.join(f)).unwrap()).unwrap()).await.unwrap();
    }
    flow(&storage, "system", "a3t-projects-install", json!({})).await;
    for org in ["acme", "globex"] {
        flow(&storage, "a3t", "a3t-orgs", json!({ "action": "create", "org": org })).await;
        flow(&storage, "a3t", "a3t-orgs", json!({ "action": "set_member", "org": org, "user_id": "root", "role": "admin" })).await;
    }
    for (org, name) in [("acme", "web"), ("acme", "api"), ("globex", "crm")] {
        flow(&storage, "a3t", "a3t-projects", json!({ "action": "create", "owner_kind": "org", "owner": org, "name": name })).await;
    }
    // Set through the flow, as a superadmin; an org's own admin can't raise it.
    flow(&storage, "a3t", "a3t-orgs", json!({ "action": "set_quota", "org": "acme", "max_active_runs": 2 })).await;
    let admin = flowr_policy::Principal::human("root2", vec!["Admin".to_string()]);
    let req = ExecutionRequest::new("a3t", "a3t-orgs").with_input(json!({ "action": "set_quota", "org": "acme", "max_active_runs": 99 })).with_principal(admin);
    let st = flowr_ce::execute_workflow(storage.clone(), &reqwest::Client::new(), req).await.unwrap();
    assert_eq!(st.output.clone().unwrap_or_default()["ok"], json!(false), "{st:?}");
    flowr_ce::access::install(storage.clone());
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let admit = |ns: &'static str| {
        let storage = storage.clone();
        async move { flowr_ce::access::admit(storage.as_ref(), ns).await }
    };
    assert!(admit("acme-web").await.is_ok(), "under the quota");

    // Two active runs, in different projects of the org, reach it.
    let engine = storage.engine().unwrap();
    for ns in ["acme-web", "acme-api"] {
        let run = pending(ns);
        let task = NewTask::start(&run);
        engine.start(run, task).await.unwrap();
    }
    let refused = admit("acme-web").await.unwrap_err();
    assert!(refused.contains("limit of 2 active runs") && refused.contains("acme"), "{refused}");
    assert!(admit("acme-api").await.is_err(), "across the org's projects");

    // Other orgs, unknown projects and the platform go on.
    assert!(admit("globex-crm").await.is_ok());
    assert!(admit("a3t").await.is_ok());
    assert!(admit("nobody-here").await.is_ok());
}
