//! Integration tests for WorkflowExecutor — proves end-to-end CNCF workflow execution
//! with no legacy types (no StepDefinition, no TaskDefinition, no old Executor)

use std::sync::Arc;
use serde_json::{json, Value};
use swf_parser::parse_yaml;
use swf_runtime::{ExecutionContext, RuntimeInfo, WorkflowExecutor};
use swf_runtime::traits::{TaskHandler, TaskResult};

fn make_ctx() -> ExecutionContext {
    ExecutionContext {
        limits: Default::default(),
        instance_id: "test-001".to_string(),
        namespace: "test".to_string(),
        name: "test".to_string(),
        version: "1.0.0".to_string(),
        context: Value::Null,
        secrets: Default::default(),
        runtime: RuntimeInfo::default(),
        storage: None,
        http_client: None,
        extensions: Vec::new(),
        eval_fn: None,
        principal: swf_runtime::default_flow_principal(),
            template_meta: Default::default(),
    }
}

#[tokio::test]
async fn test_sequential_set_tasks() {
    let yaml = r#"
document:
  dsl: '1.0.3'
  namespace: test
  name: sequential
  version: '1.0.0'
do:
  - init:
      set:
        value: 10
  - double:
      set:
        value: 20
  - result:
      set:
        final: done
"#;
    let wf = parse_yaml(yaml).unwrap();
    let executor = WorkflowExecutor::new();
    let mut ctx = make_ctx();

    let result = executor.execute(&wf, json!({}), &mut ctx).await.unwrap();
    assert_eq!(result["final"], "done");
}

#[tokio::test]
async fn test_if_condition_skips_task() {
    let yaml = r#"
document:
  dsl: '1.0.3'
  namespace: test
  name: if-skip
  version: '1.0.0'
do:
  - always:
      set:
        ran: true
  - never:
      if: "${ false }"
      set:
        should_not_run: true
  - result:
      set:
        done: true
"#;
    let wf = parse_yaml(yaml).unwrap();

    // Provide a simple eval function that handles "${ false }" → false
    let mut executor = WorkflowExecutor::new();
    executor.set_eval_fn(|_input, expr, _ctx| {
        if expr.contains("false") {
            Ok(Value::Bool(false))
        } else if expr.contains("true") {
            Ok(Value::Bool(true))
        } else {
            Ok(Value::String(expr.to_string()))
        }
    });

    let mut ctx = make_ctx();
    let result = executor.execute(&wf, json!({}), &mut ctx).await.unwrap();
    assert_eq!(result["done"], true);
    assert!(result.get("should_not_run").is_none());
}

#[tokio::test]
async fn test_then_end_stops_execution() {
    let yaml = r#"
document:
  dsl: '1.0.3'
  namespace: test
  name: early-exit
  version: '1.0.0'
do:
  - first:
      set:
        step: 1
      then: end
  - second:
      set:
        step: 2
"#;
    let wf = parse_yaml(yaml).unwrap();
    let executor = WorkflowExecutor::new();
    let mut ctx = make_ctx();

    let result = executor.execute(&wf, json!({}), &mut ctx).await.unwrap();
    assert_eq!(result["step"], 1);
}

#[tokio::test]
async fn test_do_block_sequential() {
    let yaml = r#"
document:
  dsl: '1.0.3'
  namespace: test
  name: do-test
  version: '1.0.0'
do:
  - nested:
      do:
        - inner1:
            set:
              a: 1
        - inner2:
            set:
              b: 2
"#;
    let wf = parse_yaml(yaml).unwrap();
    let executor = WorkflowExecutor::new();
    let mut ctx = make_ctx();

    let result = executor.execute(&wf, json!({}), &mut ctx).await.unwrap();
    assert_eq!(result["b"], 2);
}

#[tokio::test]
async fn test_raise_produces_error() {
    let yaml = r#"
document:
  dsl: '1.0.3'
  namespace: test
  name: raise-test
  version: '1.0.0'
do:
  - fail:
      raise:
        error:
          type: https://example.com/errors/test
          status: 400
          title: Test Error
          detail: This should fail
"#;
    let wf = parse_yaml(yaml).unwrap();
    let executor = WorkflowExecutor::new();
    let mut ctx = make_ctx();

    let err = executor.execute(&wf, json!({}), &mut ctx).await.unwrap_err();
    let msg = err.to_string(); println!("ERROR: {}", msg); assert!(msg.contains("Boom") || msg.contains("Test Error") || msg.contains("This should fail"));
    assert_eq!(err.error_type_uri(), "https://example.com/errors/test");
}

#[tokio::test]
async fn test_try_catches_error() {
    let yaml = r#"
document:
  dsl: '1.0.3'
  namespace: test
  name: try-catch
  version: '1.0.0'
do:
  - safe:
      try:
        do:
          - willFail:
              raise:
                error:
                  type: https://example.com/error
                  status: 500
                  title: Boom
        catch:
          do:
            - recover:
                set:
                  caught: true
"#;
    let wf = parse_yaml(yaml).unwrap();
    let executor = WorkflowExecutor::new();
    let mut ctx = make_ctx();

    let result = executor.execute(&wf, json!({}), &mut ctx).await.unwrap();
    assert_eq!(result["caught"], true);
}

#[tokio::test]
async fn test_switch_selects_case() {
    let yaml = r#"
document:
  dsl: '1.0.3'
  namespace: test
  name: switch-test
  version: '1.0.0'
do:
  - route:
      switch:
        - matchA:
            when: "${ false }"
            then: end
        - fallback:
            then: continue
  - result:
      set:
        reached: true
"#;
    let wf = parse_yaml(yaml).unwrap();
    let mut executor = WorkflowExecutor::new();
    executor.set_eval_fn(|_input, expr, _ctx| {
        if expr.contains("false") { Ok(Value::Bool(false)) }
        else { Ok(Value::Bool(true)) }
    });
    let mut ctx = make_ctx();

    let result = executor.execute(&wf, json!({}), &mut ctx).await.unwrap();
    assert_eq!(result["reached"], true);
}

#[tokio::test]
async fn test_unregistered_handler_returns_error() {
    let yaml = r#"
document:
  dsl: '1.0.3'
  namespace: test
  name: no-handler
  version: '1.0.0'
do:
  - callSomething:
      call: http
      with:
        method: GET
        endpoint:
          uri: https://example.com
"#;
    let wf = parse_yaml(yaml).unwrap();
    let executor = WorkflowExecutor::new();
    let mut ctx = make_ctx();

    let err = executor.execute(&wf, json!({}), &mut ctx).await.unwrap_err();
    assert!(err.to_string().contains("no handler registered"));
    assert!(err.to_string().contains("call:http"));
}

#[tokio::test]
async fn test_input_schema_validation_rejects() {
    let yaml = r#"
document:
  dsl: '1.0.3'
  namespace: test
  name: schema-reject
  version: '1.0.0'
input:
  schema:
    type: object
    required: [name]
do:
  - greet:
      set:
        message: hello
"#;
    let wf = parse_yaml(yaml).unwrap();
    let executor = WorkflowExecutor::new();
    let mut ctx = make_ctx();

    let err = executor.execute(&wf, json!({}), &mut ctx).await.unwrap_err();
    assert!(err.to_string().contains("validation"));
    assert!(err.to_string().contains("name"));
}

/// Mock handler that captures the resolved `with` values and returns them
struct CaptureWithHandler {
    task_type: String,
}

#[async_trait::async_trait]
impl TaskHandler for CaptureWithHandler {
    fn task_type(&self) -> &str {
        &self.task_type
    }

    async fn execute(
        &self,
        task: &swf_parser::NamedTask,
        _input: Value,
        _ctx: &mut ExecutionContext,
    ) -> swf_runtime::error::Result<TaskResult> {
        // Return the resolved `with` as the output so tests can inspect it
        let with = match &task.task {
            swf_parser::Task::Call(c) => c.with.clone().unwrap_or(json!({})),
            _ => json!({}),
        };
        Ok(TaskResult::new(with))
    }
}

#[tokio::test]
async fn test_call_with_expressions_evaluated() {
    let yaml = r#"
document:
  dsl: '1.0.3'
  namespace: test
  name: with-eval
  version: '1.0.0'
do:
  - setup:
      set:
        user_name: Alice
        count: 42
      export:
        as: "."
  - fetch:
      call: myapi
      with:
        greeting: "hello ${ $context.user_name }"
        total: "${ $context.count }"
        nested:
          key: "${ $context.user_name }"
      export:
        as: "."
"#;
    let wf = parse_yaml(yaml).unwrap();
    let mut executor = WorkflowExecutor::new();

    // Wire a real jq engine
    executor.set_eval_fn(|input, expr, context| {
        let engine = flowr_expr::ExpressionEngine::new();
        let scope = flowr_expr::Scope::builder().context(context.clone()).build();
        engine.eval(input, expr, &scope).map_err(|e| e.to_string())
    });

    // Register mock handler
    executor.register_handler(Arc::new(CaptureWithHandler {
        task_type: "call:myapi".to_string(),
    }));

    let mut ctx = make_ctx();
    let result = executor.execute(&wf, json!({}), &mut ctx).await.unwrap();

    // The handler should have received resolved values, not raw ${ } strings
    assert_eq!(result["greeting"], "hello Alice");
    assert_eq!(result["total"], 42);
    assert_eq!(result["nested"]["key"], "Alice");
}

#[tokio::test]
async fn test_session_restore_pattern() {
    // Reproduce the exact session restore pattern from agent-template
    let yaml = r#"
document:
  dsl: '1.0.3'
  namespace: test
  name: session-restore
  version: '1.0.0'
do:
  - init:
      set:
        _messages: []
        session_id: "test-session"
      export:
        as: "."

  - load_session:
      call: state:get
      with:
        key: "agent:session:${ $context.session_id }"
      export:
        as: "{_session: .}"

  - restore_session:
      set:
        _messages: "${ (if $context._session then ($context._session.messages // []) else [] end) }"
      export:
        as: "."

  - check:
      set:
        messages_is_array: "${ ($context._messages | type) == \"array\" }"
        messages_length: "${ $context._messages | length }"
      export:
        as: "."
"#;
    let wf = parse_yaml(yaml).unwrap();
    let mut executor = WorkflowExecutor::new();

    executor.set_eval_fn(|input, expr, context| {
        let engine = flowr_expr::ExpressionEngine::new();
        let scope = flowr_expr::Scope::builder().context(context.clone()).build();
        engine.eval(input, expr, &scope).map_err(|e| e.to_string())
    });

    // Register a mock state:get that returns null (simulating no existing session)
    executor.register_handler(Arc::new(CaptureWithHandler {
        task_type: "call:state:get".to_string(),
    }));

    let mut ctx = make_ctx();
    let result = executor.execute(&wf, json!({}), &mut ctx).await.unwrap();

    // _messages should be an empty array, NOT a string
    assert_eq!(result["messages_is_array"], true, "messages should be array, got: {:?}", result);
    assert_eq!(result["messages_length"], 0);
}

// ── ST-1: a catch handler sees what it caught, and filters by type ──

#[tokio::test]
async fn catch_exposes_the_error_as_context_error() {
    let yaml = r#"
document:
  dsl: '1.0.3'
  namespace: test
  name: try-error
  version: '1.0.0'
do:
  - safe:
      try:
        do:
          - willFail:
              raise:
                error:
                  type: https://a3t.io/errors/input
                  status: 400
                  title: Bad input
                  detail: name is required
        catch:
          do:
            - recover:
                set:
                  caught: true
"#;
    let wf = parse_yaml(yaml).unwrap();
    let executor = WorkflowExecutor::new();
    let mut ctx = make_ctx();
    let result = executor.execute(&wf, json!({}), &mut ctx).await.unwrap();
    assert_eq!(result["caught"], true);
    let e = &ctx.context["error"];
    assert_eq!(e["type"], "https://a3t.io/errors/input");
    assert_eq!(e["status"], 400);
    assert_eq!(e["title"], "Bad input");
    assert_eq!(e["detail"], "name is required");
}

#[tokio::test]
async fn catch_filter_lets_other_errors_through() {
    let yaml = r#"
document:
  dsl: '1.0.3'
  namespace: test
  name: try-filter
  version: '1.0.0'
do:
  - safe:
      try:
        do:
          - willFail:
              raise:
                error:
                  type: https://serverlessworkflow.io/spec/1.0.0/errors/authorization
                  status: 403
                  title: Denied
        catch:
          errors:
            with:
              type: timeout
          do:
            - recover:
                set:
                  caught: true
"#;
    let wf = parse_yaml(yaml).unwrap();
    let executor = WorkflowExecutor::new();
    let mut ctx = make_ctx();
    let err = executor.execute(&wf, json!({}), &mut ctx).await.unwrap_err();
    assert!(err.error_type_uri().ends_with("/authorization"));

    // The same filter by last segment or status catches it.
    let caught = yaml.replace("type: timeout", "status: 403");
    let wf = parse_yaml(&caught).unwrap();
    let mut ctx = make_ctx();
    let result = executor.execute(&wf, json!({}), &mut ctx).await.unwrap();
    assert_eq!(result["caught"], true);
}

#[test]
fn error_object_maps_runtime_errors() {
    use swf_runtime::RuntimeError;
    let o = swf_runtime::executor::error_object(&RuntimeError::Timeout("call:llm after 30s".into()));
    assert_eq!(o["status"], 408);
    assert!(o["type"].as_str().unwrap().ends_with("/timeout"));
    let o = swf_runtime::executor::error_object(&RuntimeError::Denied {
        task: "send".into(), reason: "egress not allowed".into(), policy_ids: vec!["P-12".into()],
    });
    assert_eq!(o["status"], 403);
    assert_eq!(o["instance"], "send");
    assert!(o["detail"].as_str().unwrap().contains("P-12"));
}

/// Stands in for run:workflow: answers like system/a3t-error-handler, and
/// records the input it was given.
struct FakeErrorHandler {
    seen: std::sync::Mutex<Vec<Value>>,
}

#[async_trait::async_trait]
impl TaskHandler for FakeErrorHandler {
    fn task_type(&self) -> &str { "run:workflow" }
    async fn execute(
        &self,
        task: &swf_parser::NamedTask,
        _input: Value,
        _ctx: &mut ExecutionContext,
    ) -> swf_runtime::error::Result<TaskResult> {
        let inp = match &task.task {
            swf_parser::Task::Run(swf_parser::RunTask::Workflow(w)) => w.input.clone().unwrap_or_default(),
            _ => json!({}),
        };
        self.seen.lock().unwrap().push(inp);
        Ok(TaskResult::new(json!({
            "class": "input", "status": 400, "title": "Invalid input",
            "user_message": "Couldn't save the agent: name is required.", "ref": "run-1"
        })))
    }
}

fn failing_flow(on_error: &str, error_type: &str) -> String {
    format!(r#"
document:
  dsl: '1.0.3'
  namespace: a3t
  name: a3t-agents
  version: '1.0.0'
metadata:
  a3t:
    onError: {on_error}
do:
  - save:
      raise:
        error:
          type: {error_type}
          status: 400
          title: Bad
          detail: name missing
"#)
}

#[tokio::test]
async fn on_error_hook_replaces_the_failure_with_the_handlers_message() {
    let handler = Arc::new(FakeErrorHandler { seen: Default::default() });
    let mut executor = WorkflowExecutor::new();
    executor.register_handler(handler.clone());
    let wf = parse_yaml(&failing_flow("system/a3t-error-handler", "https://serverlessworkflow.io/spec/1.0.0/errors/validation")).unwrap();
    let mut ctx = make_ctx();
    let err = executor.execute(&wf, json!({}), &mut ctx).await.unwrap_err();
    assert_eq!(err.error_type_uri(), "https://a3t.io/errors/input");
    assert!(err.to_string().contains("Couldn't save the agent: name is required."));
    let seen = handler.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0]["flow"], "a3t/a3t-agents");
    assert_eq!(seen[0]["error"]["detail"], "name missing");
    assert_eq!(seen[0]["run_id"], "test-001");
}

#[tokio::test]
async fn on_error_hook_passes_an_already_handled_error_through() {
    let handler = Arc::new(FakeErrorHandler { seen: Default::default() });
    let mut executor = WorkflowExecutor::new();
    executor.register_handler(handler.clone());
    let wf = parse_yaml(&failing_flow("system/a3t-error-handler", "https://a3t.io/errors/input")).unwrap();
    let mut ctx = make_ctx();
    let err = executor.execute(&wf, json!({}), &mut ctx).await.unwrap_err();
    assert_eq!(err.error_type_uri(), "https://a3t.io/errors/input");
    assert!(handler.seen.lock().unwrap().is_empty(), "logged once, by the inner flow");
}

#[tokio::test]
async fn without_a_handler_the_original_error_stands() {
    let executor = WorkflowExecutor::new(); // no run:workflow registered
    let wf = parse_yaml(&failing_flow("system/a3t-error-handler", "https://example.com/x")).unwrap();
    let mut ctx = make_ctx();
    let err = executor.execute(&wf, json!({}), &mut ctx).await.unwrap_err();
    assert_eq!(err.error_type_uri(), "https://example.com/x");
}

/// Fails its first `fail_times` calls with `err`, then answers {ok: true}.
struct Flaky {
    calls: std::sync::atomic::AtomicU32,
    fail_times: u32,
    denied: bool,
}

#[async_trait::async_trait]
impl TaskHandler for Flaky {
    fn task_type(&self) -> &str { "call:llm" }
    async fn execute(&self, task: &swf_parser::NamedTask, _i: Value, _c: &mut ExecutionContext) -> swf_runtime::error::Result<TaskResult> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        if n <= self.fail_times {
            return Err(if self.denied {
                swf_runtime::RuntimeError::Denied { task: task.name.clone(), reason: "no".into(), policy_ids: vec![] }
            } else {
                swf_runtime::RuntimeError::Communication("provider answered 429 Too Many Requests".into())
            });
        }
        Ok(TaskResult::new(json!({ "ok": true })))
    }
}

const RETRY_FLOW: &str = r#"
document:
  dsl: '1.0.3'
  namespace: test
  name: retry
  version: '1.0.0'
do:
  - ask:
      try:
        do:
          - llm:
              call: llm
        catch:
          retry:
            limit:
              attempt:
                count: 3
          do:
            - gave-up:
                set:
                  gave_up: true
"#;

#[tokio::test]
async fn catch_retry_runs_again_until_it_succeeds() {
    let flaky = Arc::new(Flaky { calls: Default::default(), fail_times: 2, denied: false });
    let mut executor = WorkflowExecutor::new();
    executor.register_handler(flaky.clone());
    let mut ctx = make_ctx();
    let out = executor.execute(&parse_yaml(RETRY_FLOW).unwrap(), json!({}), &mut ctx).await.unwrap();
    assert_eq!(out["ok"], true);
    assert_eq!(flaky.calls.load(std::sync::atomic::Ordering::SeqCst), 3, "1 try + 2 retries");
}

#[tokio::test]
async fn catch_retry_gives_up_after_its_limit_and_reports_attempts() {
    let flaky = Arc::new(Flaky { calls: Default::default(), fail_times: 99, denied: false });
    let mut executor = WorkflowExecutor::new();
    executor.register_handler(flaky.clone());
    let mut ctx = make_ctx();
    let out = executor.execute(&parse_yaml(RETRY_FLOW).unwrap(), json!({}), &mut ctx).await.unwrap();
    assert_eq!(out["gave_up"], true);
    assert_eq!(flaky.calls.load(std::sync::atomic::Ordering::SeqCst), 4, "1 try + 3 retries");
    assert_eq!(ctx.context["error"]["attempts"], 4);
}

#[tokio::test]
async fn a_denial_is_never_retried() {
    let flaky = Arc::new(Flaky { calls: Default::default(), fail_times: 99, denied: true });
    let mut executor = WorkflowExecutor::new();
    executor.register_handler(flaky.clone());
    let mut ctx = make_ctx();
    let out = executor.execute(&parse_yaml(RETRY_FLOW).unwrap(), json!({}), &mut ctx).await.unwrap();
    assert_eq!(out["gave_up"], true);
    assert_eq!(flaky.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}
