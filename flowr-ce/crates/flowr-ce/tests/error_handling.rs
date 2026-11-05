//! UX steel thread ST-1, step 1: a failure in any A3T flow is handled the same
//! way. Real executor, real `run:workflow` and `call:lancedb`, a temporary
//! platform DB. A fault flow declares `metadata.a3t.onError` like every A3T
//! flow and raises each error class on demand (fault injection, ST-1.8):
//!
//! - the run fails with the plain-language message, not the raw error;
//! - exactly one row lands in `a3t_errors`, with the right class;
//! - the log is readable by a superadmin only;
//! - an A3T flow that declares the hook still answers normally.

use std::sync::Arc;

use flowr_storage::StorageBackend;
use flowr_types::{ExecutionRequest, ExecutionStatus, Resource, WorkflowRecord};
use serde_json::{json, Value};

const FLOWS: &[&str] = &[
    "system/a3t_error_handler.yaml",
    "a3t/a3t_error_log.yaml",
    "a3t/agents_crud.yaml",
    "system/a3t_schema_install.yaml",
];

/// Raises the fault named in `.fault`, as each kind of failure reaches a flow.
const FAULT_FLOW: &str = r#"
document:
  dsl: 1.0.3
  namespace: a3t
  name: st1-fault
  version: 1.0.0
metadata:
  a3t:
    onError: system/a3t-error-handler
    durability: transient
do:
- pick:
    switch:
    - timeout: { when: '${ .fault == "timeout" }', then: timeout }
    - rate: { when: '${ .fault == "llm-429" }', then: rate }
    - input: { when: '${ .fault == "bad-input" }', then: bad-input }
    - denied: { when: '${ .fault == "denial" }', then: denial }
    - bug: { when: '${ .fault == "bug" }', then: bug }
    - default: { then: fine }
- fine:
    set: { ok: true }
    then: end
- timeout:
    raise:
      error: { type: 'https://serverlessworkflow.io/spec/1.0.0/errors/timeout', status: 408, title: Timed out, detail: 'call:llm took longer than 30s' }
- rate:
    raise:
      error: { type: 'https://serverlessworkflow.io/spec/1.0.0/errors/communication', status: 502, title: LLM error, detail: 'provider answered 429 Too Many Requests' }
- bad-input:
    raise:
      error: { type: 'https://serverlessworkflow.io/spec/1.0.0/errors/validation', status: 400, title: Invalid input, detail: 'name is required' }
- denial:
    raise:
      error: { type: 'https://serverlessworkflow.io/spec/1.0.0/errors/authorization', status: 403, title: Denied, detail: 'email outside acme.com (policies: P-12)' }
- bug:
    raise:
      error: { type: 'https://serverlessworkflow.io/spec/1.0.0/errors/runtime', status: 500, title: Boom, detail: 'index out of range' }
"#;

const SUPER: &[&str] = &["SuperAdmin", "Admin"];
const DEV: &[&str] = &["Admin", "Operator"];

struct World {
    storage: Arc<dyn StorageBackend>,
    http: reqwest::Client,
    _dir: tempfile::TempDir,
}

impl World {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let storage = flowr_ce::init_storage(dir.path().to_str().unwrap()).await.unwrap();
        let db = dir.path().join("platform.lance");
        storage.resources().create(
            Resource::new("a3t-platform-db", "lancedb", json!({ "db_path": db.to_str().unwrap() })).with_namespace("a3t"),
        ).await.unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../a3t/backend/flows");
        for f in FLOWS {
            let yaml = std::fs::read_to_string(root.join(f)).unwrap();
            storage.workflows().register(WorkflowRecord::from_yaml(&yaml).unwrap()).await.unwrap();
        }
        storage.workflows().register(WorkflowRecord::from_yaml(FAULT_FLOW).unwrap()).await.unwrap();
        World { storage, http: reqwest::Client::new(), _dir: dir }
    }

    async fn run(&self, roles: &[&str], flow: &str, input: Value) -> (ExecutionStatus, Value, String) {
        let principal = flowr_policy::Principal::human("usr-test", roles.iter().map(|r| r.to_string()).collect());
        let ns = if flow == "a3t-schema-install" { "system" } else { "a3t" };
        let req = ExecutionRequest::new(ns, flow).with_input(input).with_principal(principal);
        let state = flowr_ce::execute_workflow(self.storage.clone(), &self.http, req).await.unwrap();
        (state.status, state.output.unwrap_or(Value::Null), state.error.unwrap_or_default())
    }

    async fn errors(&self, filter: Value) -> Value {
        let mut input = json!({ "action": "list" });
        input.as_object_mut().unwrap().extend(filter.as_object().unwrap().clone());
        let (status, out, err) = self.run(SUPER, "a3t-error-log", input).await;
        assert_eq!(status, ExecutionStatus::Completed, "error log: {err}");
        assert_eq!(out["ok"], true, "error log: {out}");
        out
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_is_classified_logged_once_and_shown_in_plain_words() {
    let w = World::new().await;

    // (fault, class, words the person sees)
    let cases = [
        ("timeout", "transient", "didn't answer in time"),
        ("llm-429", "transient", "didn't answer in time"),
        ("bad-input", "input", "Couldn't do that: name is required"),
        ("denial", "policy", "Not allowed: email outside acme.com"),
        ("bug", "bug", "Something failed on our side. Reference:"),
    ];
    for (fault, class, words) in cases {
        let (status, _, err) = w.run(DEV, "st1-fault", json!({ "fault": fault })).await;
        assert_eq!(status, ExecutionStatus::Failed, "{fault} should fail the run");
        assert!(err.contains(&format!("https://a3t.io/errors/{class}")), "{fault}: class {class} expected in {err}");
        assert!(err.contains(words), "{fault}: expected “{words}” in {err}");
        assert!(!err.contains("index out of range") || fault != "bug", "the raw detail of a bug never reaches the user: {err}");
    }

    // One row per failure, each with its class, newest first.
    let log = w.errors(json!({ "flow": "a3t/st1-fault" })).await;
    let rows = log["errors"].as_array().unwrap();
    assert_eq!(rows.len(), cases.len(), "exactly one row per failure: {log}");
    assert_eq!(log["counts"]["transient"], 2);
    assert_eq!(log["counts"]["input"], 1);
    assert_eq!(log["counts"]["policy"], 1);
    assert_eq!(log["counts"]["bug"], 1);
    let bug = rows.iter().find(|r| r["class"] == "bug").unwrap();
    assert_eq!(bug["detail"], "index out of range", "the log keeps the raw detail for P0");
    assert_eq!(bug["flow"], "a3t/st1-fault");
    assert_eq!(bug["principal"], "usr-test");
    assert!(bug["user_message"].as_str().unwrap().contains(bug["run_id"].as_str().unwrap()), "the reference is the run id");

    // A filter narrows it.
    let policy = w.errors(json!({ "class": "policy" })).await;
    assert_eq!(policy["errors"].as_array().unwrap().len(), 1);

    // Only a superadmin reads the log.
    let (status, out, _) = w.run(DEV, "a3t-error-log", json!({ "action": "list" })).await;
    assert_eq!(status, ExecutionStatus::Completed);
    assert_eq!(out["ok"], false);
    assert!(out["error"].as_str().unwrap().contains("superadmin"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_flow_that_declares_the_hook_still_answers_normally() {
    let w = World::new().await;
    let (status, out, err) = w.run(DEV, "st1-fault", json!({ "fault": "none" })).await;
    assert_eq!(status, ExecutionStatus::Completed, "{err}");
    assert_eq!(out["ok"], true);
    // A real A3T flow with the hook. Before the schema install its table is
    // missing: that is our fault, not a busy service, so it is a bug, not
    // "try again in a minute".
    let list = json!({ "input": { "action": "list", "org_id": "org-default" } });
    let (status, _, err) = w.run(DEV, "a3t-agents", list.clone()).await;
    assert_eq!(status, ExecutionStatus::Failed);
    assert!(err.contains("https://a3t.io/errors/bug") && err.contains("Reference:"), "{err}");
    let log = w.errors(json!({ "flow": "a3t/a3t-agents" })).await;
    assert_eq!(log["errors"].as_array().unwrap().len(), 1, "{log}");

    // After the install it answers, and nothing more is logged.
    let (status, _, err) = w.run(SUPER, "a3t-schema-install", json!({})).await;
    assert_eq!(status, ExecutionStatus::Completed, "schema install: {err}");
    let (status, _, err) = w.run(DEV, "a3t-agents", list).await;
    assert_eq!(status, ExecutionStatus::Completed, "a3t-agents list after install: {err}");
    let log = w.errors(json!({})).await;
    assert_eq!(log["errors"].as_array().unwrap().len(), 1, "{log}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_log_answers_empty_before_anything_failed() {
    let w = World::new().await;
    let log = w.errors(json!({})).await;
    assert_eq!(log["errors"], json!([]));
}
