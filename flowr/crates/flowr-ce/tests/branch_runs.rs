//! AC-6 and AC-7 with the real executor: a run on a memory branch keeps its branch through
//! subflows and the run row, and no call leaves A3T from it.

use std::sync::Arc;

use flowr_storage::StorageBackend;
use flowr_types::{ExecutionRequest, ExecutionStatus, Resource, WorkflowRecord};
use serde_json::{json, Value};

const CALLS_OUT: &str = r#"
document:
  dsl: 1.0.3
  namespace: t
  name: calls-out
  version: 1.0.0
do:
- out:
    call: http
    with:
      method: get
      endpoint: http://127.0.0.1:9/never
"#;

const PARENT: &str = r#"
document:
  dsl: 1.0.3
  namespace: t
  name: parent
  version: 1.0.0
do:
- nested:
    run:
      workflow:
        namespace: t
        name: calls-out
        version: 1.0.0
"#;

async fn world() -> (tempfile::TempDir, Arc<dyn StorageBackend>) {
    let dir = tempfile::tempdir().unwrap();
    let storage = flowr_ce::init_storage(dir.path().to_str().unwrap()).await.unwrap();
    let db = dir.path().join("platform.lance");
    storage.resources().create(Resource::new("a3t-platform-db", "lancedb", json!({ "db_path": db.to_str().unwrap() })).with_namespace("a3t")).await.unwrap();
    for y in [CALLS_OUT, PARENT] {
        storage.workflows().register(WorkflowRecord::from_yaml(y).unwrap()).await.unwrap();
    }
    (dir, storage)
}

fn alice() -> flowr_policy::Principal {
    flowr_policy::Principal::human("alice", vec!["Admin".to_string()])
}

async fn decisions(storage: &Arc<dyn StorageBackend>) -> Vec<Value> {
    use flowr_ce::handlers::call_lancedb::{DEFAULT_PLATFORM_DB_PATH, PLATFORM_DB_RESOURCE};
    let db = flowr_ce::platform_db::open_platform_db(storage.as_ref(), Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let batches = db.query("gateway_decisions", None, 100).await.unwrap();
    let rows = flowr_ce::handlers::call_lancedb::batches_to_json(&batches);
    serde_json::to_value(rows).unwrap().as_array().cloned().unwrap_or_default()
}

#[test]
fn the_branch_survives_the_queue_and_an_agent_inherits_it() {
    let p = alice().on_branch("try").unwrap();
    let back: flowr_policy::Principal = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
    assert_eq!(back.memory_branch(), Some("try"));
    assert!(!serde_json::to_string(&alice()).unwrap().contains("memory_branch"), "a run on main looks as before");
    let agent = flowr_policy::Principal::agent_for(flowr_policy::AgentId("a".into()), &p, None, None, None);
    assert_eq!(agent.memory_branch(), Some("try"));
    assert!(alice().on_branch("no spaces").is_err());
    assert_eq!(flowr_policy::Principal::service("s").on_branch("try").unwrap().memory_branch(), Some("try"));
}

#[test]
fn a_run_names_its_branch_and_main_means_none() {
    use flowr_mcp_core::flow_base::run_principal;
    assert_eq!(run_principal(&alice(), &json!({ "memory_branch": "try" })).unwrap().memory_branch(), Some("try"));
    assert_eq!(run_principal(&alice(), &json!({ "memory_branch": "main" })).unwrap().memory_branch(), None);
    assert_eq!(run_principal(&alice(), &json!({})).unwrap().memory_branch(), None);
    assert!(run_principal(&alice(), &json!({ "memory_branch": "a\"b" })).is_err());
}

#[test]
fn only_the_model_is_allowed_out_of_a_branch() {
    use flowr_ce::gateway::outbound_blocked;
    let p = alice().on_branch("try").unwrap();
    for t in ["call:http", "call:mcp"] {
        let reason = outbound_blocked(&p, t, "http://x").unwrap_or_else(|| panic!("{t} allowed"));
        assert!(reason.contains("try") && reason.contains("http://x"), "{reason}");
    }
    assert!(outbound_blocked(&p, "call:llm", "llm:x").is_none());
    assert!(outbound_blocked(&alice(), "call:http", "http://x").is_none(), "main in production");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_run_is_refused_outbound_with_a_plain_message_and_a_decision_row() {
    let (_dir, storage) = world().await;
    // Through a subflow: the branch must not be lost on the way.
    let req = ExecutionRequest::new("t", "parent").with_input(json!({})).with_principal(alice().on_branch("try").unwrap());
    let st = flowr_ce::execute_workflow(storage.clone(), &reqwest::Client::new(), req).await.unwrap();
    assert_eq!(st.status, ExecutionStatus::Failed, "{st:?}");
    let error = st.error.clone().unwrap_or_default();
    assert!(error.contains("memory branch") && error.contains("127.0.0.1:9/never"), "{error}");
    let rows = decisions(&storage).await;
    let row = rows.iter().find(|r| r["target"].as_str().is_some_and(|t| t.contains("/never"))).unwrap_or_else(|| panic!("no decision row in {rows:?}"));
    assert_eq!(row["decision"], json!("deny"), "{row}");
    assert_eq!(row["memory_branch"], json!("try"), "{row}");
    assert_eq!(row["environment"], json!("prod"), "{row}");

    // On main the same flow does try the call (and fails on the closed port, not on a refusal).
    let req = ExecutionRequest::new("t", "calls-out").with_input(json!({})).with_principal(alice());
    let st = flowr_ce::execute_workflow(storage.clone(), &reqwest::Client::new(), req).await.unwrap();
    assert!(!st.error.clone().unwrap_or_default().contains("memory branch"), "{st:?}");
}

/// AC-8: a looping flow stops at its step limit, which it can lower and a subflow shares.
#[tokio::test(flavor = "multi_thread")]
async fn a_looping_run_stops_at_its_step_limit() {
    let (_dir, storage) = world().await;
    const LOOP: &str = r#"
document:
  dsl: 1.0.3
  namespace: t
  name: looper
  version: 1.0.0
metadata:
  a3t:
    limits:
      max_steps: 25
do:
- again:
    set:
      n: 1
    then: again
"#;
    const CALLS_LOOP: &str = r#"
document:
  dsl: 1.0.3
  namespace: t
  name: calls-looper
  version: 1.0.0
do:
- nested:
    run:
      workflow:
        namespace: t
        name: looper
        version: 1.0.0
"#;
    for y in [LOOP, CALLS_LOOP] {
        storage.workflows().register(WorkflowRecord::from_yaml(y).unwrap()).await.unwrap();
    }
    for flow in ["looper", "calls-looper"] {
        let req = ExecutionRequest::new("t", flow).with_input(json!({})).with_principal(alice());
        let st = flowr_ce::execute_workflow(storage.clone(), &reqwest::Client::new(), req).await.unwrap();
        assert_eq!(st.status, ExecutionStatus::Failed, "{flow}: {st:?}");
        assert!(st.error.clone().unwrap_or_default().contains("limit of 25 steps"), "{flow}: {:?}", st.error);
    }
}
