//! The gateway (`.ai/A3T_CLUSTER_ARCHITECTURE.md` §10, G2 + G3): every outbound call
//! of a runtime goes through one pool, `gateway`, which decides and acts.
//!
//! **Client (runtimes, `FLOWR_GATEWAY=pool`).** The outbound handlers
//! ([`OUTBOUND`]) are replaced by [`GatewayClient`]: the step starts a run of
//! `system-gateway/call` (routed to pool `gateway`, see [`crate::router`]) carrying
//! the resolved task, the step input and the caller, and waits for its outcome. The
//! start is deduplicated on the caller's step key, so a step that runs again after a
//! crash finds the same gateway run and nothing is called twice. The runtime holds no
//! downstream credentials.
//!
//! **Gateway (pool `gateway`).** `system-gateway/call` is one step,
//! `call:a3t:gateway` ([`GatewayStep`]): Verify → Resolve → Decide → Act → Record.
//! Decide is a placeholder ([`GatewayPolicy`]): it allows everything and says so;
//! `FLOWR_GATEWAY_DENY` (comma-separated substrings of the target) denies, for
//! tests. Act runs the real handler in this process, as the caller (its namespace
//! and principal). Record writes a `gateway_decisions` row to the platform database.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use swf_parser::NamedTask;
use swf_runtime::context::ExecutionContext;
use swf_runtime::error::{Result, RuntimeError};
use swf_runtime::traits::{TaskHandler, TaskResult};
use tracing::{info, warn};

use flowr_storage::engine::{run_topic, Durability, NewRun, NewTask, RunPhase};
use flowr_storage::StorageBackend;

use crate::router::{GATEWAY_NAMESPACE, GATEWAY_POOL};

/// The gateway flow and its one step type.
pub const GATEWAY_FLOW: &str = "call";
pub const GATEWAY_STEP: &str = "call:a3t:gateway";

/// The handlers whose calls leave A3T.
pub const OUTBOUND: &[&str] = &["call:http", "call:mcp", "call:llm"];

/// `FLOWR_GATEWAY=pool`: this process sends its outbound calls to the gateway pool.
pub fn client_enabled() -> bool {
    std::env::var("FLOWR_GATEWAY").map(|v| v == "pool").unwrap_or(false)
}

/// Whether this process serves the gateway pool.
pub fn serves_gateway() -> bool {
    crate::engine_worker::worker_pools().iter().any(|p| p == GATEWAY_POOL)
}

/// A fresh instance of the real handler for an outbound task type.
fn real_handler(task_type: &str) -> Option<Arc<dyn TaskHandler>> {
    Some(match task_type {
        "call:http" => Arc::new(swf_runtime::handlers::call_http::HttpHandler::new()),
        "call:mcp" => Arc::new(crate::handlers::call_mcp::McpHandler::new()),
        "call:llm" => Arc::new(crate::handlers::call_llm::LlmHandler::new()),
        _ => return None,
    })
}

/// Register the gateway: the client wrappers when `FLOWR_GATEWAY=pool`, and the
/// gateway step everywhere (it only ever runs in gateway flows).
pub fn register(executor: &mut swf_runtime::executor::WorkflowExecutor) {
    for t in OUTBOUND {
        if let Some(inner) = real_handler(t) {
            if client_enabled() {
                executor.register_handler(Arc::new(GatewayClient { inner }));
            } else {
                // No gateway pool here (the console's own flows, CE): AC-7 still holds.
                executor.register_handler(Arc::new(OutboundGuard { inner }));
            }
        }
    }
    executor.register_handler(Arc::new(GatewayStep));
}

fn storage_of(ctx: &ExecutionContext) -> Option<Arc<dyn StorageBackend>> {
    ctx.storage.as_ref().and_then(|s| s.downcast_ref::<Arc<dyn StorageBackend>>().cloned())
}

// ---------------------------------------------------------- no outbound outside prod

/// AC-7 (D5): why a call may not leave A3T, if so: the run is on a memory branch, or this
/// process serves a non-production environment (`A3T_ENVIRONMENT`, default prod). LLM
/// calls stay allowed, as an agent can't run without its model; its tools can't go out.
pub fn outbound_blocked(principal: &flowr_policy::Principal, task_type: &str, target: &str) -> Option<String> {
    if task_type == "call:llm" {
        return None;
    }
    if let Some(b) = principal.memory_branch() {
        return Some(format!("This run is on memory branch \"{b}\", where calls out of A3T are off, so {target} was not called."));
    }
    let env = std::env::var("A3T_ENVIRONMENT").unwrap_or_default();
    if !env.is_empty() && env != "prod" && env != "production" {
        return Some(format!("Calls out of A3T are off in the {env} environment, so {target} was not called."));
    }
    None
}

/// [`outbound_blocked`] for a call that doesn't pass through the gateway step: refused with
/// a plain message and a decision row.
pub async fn refuse_outbound(storage: &dyn StorageBackend, principal: &flowr_policy::Principal, caller_run: &str, task_type: &str, target: &str) -> std::result::Result<(), String> {
    let Some(reason) = outbound_blocked(principal, task_type, target) else { return Ok(()) };
    let call = Call {
        task_type: task_type.to_string(),
        target: target.to_string(),
        caller_run: caller_run.to_string(),
        caller_step: swf_runtime::effects::step_key(),
        namespace: String::new(),
        flow: String::new(),
        principal: serde_json::to_value(principal).unwrap_or(Value::Null),
    };
    let decision = Decision { allow: false, reason: reason.clone() };
    record(storage, &uuid::Uuid::now_v7().to_string(), &call, "none", &decision, &Err(reason.clone()), Duration::ZERO).await;
    Err(reason)
}

/// An outbound handler in a process with no gateway pool.
pub struct OutboundGuard {
    inner: Arc<dyn TaskHandler>,
}

#[async_trait]
impl TaskHandler for OutboundGuard {
    fn task_type(&self) -> &str {
        self.inner.task_type()
    }

    fn source(&self) -> &str {
        self.inner.source()
    }

    fn policy_resource(&self, task: &NamedTask) -> flowr_policy::Resource {
        self.inner.policy_resource(task)
    }

    fn policy_action(&self) -> flowr_policy::Action {
        self.inner.policy_action()
    }

    async fn execute(&self, task: &NamedTask, input: Value, ctx: &mut ExecutionContext) -> Result<TaskResult> {
        let target = target_of(self.inner.task_type(), task);
        if let Some(reason) = outbound_blocked(&ctx.principal, self.inner.task_type(), &target) {
            match storage_of(ctx) {
                Some(s) => {
                    let _ = refuse_outbound(s.as_ref(), &ctx.principal, &ctx.instance_id, self.inner.task_type(), &target).await;
                }
                None => warn!(%target, "outbound refused; no storage to record the decision"),
            }
            return Err(RuntimeError::task_failed(&task.name, reason));
        }
        self.inner.execute(task, input, ctx).await
    }
}

// ------------------------------------------------------------------ client side

/// An outbound handler on a runtime: the call runs in the gateway pool.
pub struct GatewayClient {
    inner: Arc<dyn TaskHandler>,
}

#[async_trait]
impl TaskHandler for GatewayClient {
    fn task_type(&self) -> &str {
        self.inner.task_type()
    }

    // The runtime's own choke point still sees the real target.
    fn policy_resource(&self, task: &NamedTask) -> flowr_policy::Resource {
        self.inner.policy_resource(task)
    }

    fn policy_action(&self) -> flowr_policy::Action {
        self.inner.policy_action()
    }

    async fn execute(&self, task: &NamedTask, input: Value, ctx: &mut ExecutionContext) -> Result<TaskResult> {
        let storage = storage_of(ctx).ok_or_else(|| RuntimeError::Internal("gateway: storage not available".into()))?;
        let engine = storage
            .engine()
            .ok_or_else(|| RuntimeError::task_failed(&task.name, "gateway: this process has no engine store, so it cannot reach the gateway pool"))?;
        let step = swf_runtime::effects::step_key();
        let request = json!({
            "task_type": self.inner.task_type(),
            "task": task,
            "input": input,
            "caller": {
                "run_id": ctx.instance_id,
                "step": step,
                "namespace": ctx.namespace,
                "flow": ctx.name,
                "principal": ctx.principal,
            },
        });
        let run = NewRun {
            run_id: uuid::Uuid::now_v7(),
            namespace: GATEWAY_NAMESPACE.to_string(),
            flow: GATEWAY_FLOW.to_string(),
            version: String::new(),
            def_hash: String::new(),
            parent_run_id: uuid::Uuid::parse_str(&ctx.instance_id).ok(),
            durability: Durability::Durable,
            debug: false,
            breakpoints: None,
            principal: serde_json::to_value(&ctx.principal).unwrap_or(Value::Null),
            trigger: "gateway".into(),
            input: request,
            deadline_at: None,
            // The same gateway run for every attempt of this step.
            dedupe_key: step.as_ref().map(|k| format!("gateway:{}:{k}", ctx.instance_id)),
        };
        // Explicitly the gateway pool: a runtime has no router installed (only the
        // process holding the store does), and without one everything goes to `default`.
        let mut start = NewTask::start(&run);
        start.pool = gateway_pool_for(storage.as_ref(), &ctx.namespace).await;
        let (id, _) = engine.start(run, start).await.map_err(|e| RuntimeError::task_failed(&task.name, format!("gateway: {e}")))?;
        let mut notices = engine.subscribe(&run_topic(id));
        let t0 = Instant::now();
        loop {
            match engine.get(id).await {
                Ok(Some(row)) => match row.phase {
                    RunPhase::Completed => {
                        tracing::debug!(task = %task.name, gateway_run = %id, took_ms = t0.elapsed().as_millis() as u64, "gateway call done");
                        return Ok(TaskResult::new(row.output.unwrap_or(Value::Null)));
                    }
                    RunPhase::Faulted | RunPhase::Cancelled => {
                        return Err(RuntimeError::task_failed(&task.name, row.error.unwrap_or_else(|| "gateway call failed".into())));
                    }
                    _ => {}
                },
                Ok(None) => return Err(RuntimeError::task_failed(&task.name, format!("gateway run {id} vanished"))),
                Err(e) => warn!(gateway_run = %id, error = %e, "gateway: reading the run failed; retrying"),
            }
            tokio::select! {
                _ = notices.next() => {}
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
        }
    }
}

// ----------------------------------------------------------------- gateway side

/// The gateway's decision.
#[derive(Debug, Clone)]
pub struct Decision {
    pub allow: bool,
    pub reason: String,
}

/// Decide whether a call may go out. The policy layer (Cedar, identity) comes later;
/// until then [`PlaceholderPolicy`] is the only implementation.
pub trait GatewayPolicy: Send + Sync {
    fn decide(&self, call: &Call) -> Decision;
}

/// Allows everything, unless `FLOWR_GATEWAY_DENY` names a substring of the target.
pub struct PlaceholderPolicy;

impl GatewayPolicy for PlaceholderPolicy {
    fn decide(&self, call: &Call) -> Decision {
        if let Ok(p) = serde_json::from_value::<flowr_policy::Principal>(call.principal.clone()) {
            if let Some(reason) = outbound_blocked(&p, &call.task_type, &call.target) {
                return Decision { allow: false, reason };
            }
        }
        let deny = std::env::var("FLOWR_GATEWAY_DENY").unwrap_or_default();
        if let Some(p) = deny.split(',').map(str::trim).filter(|p| !p.is_empty()).find(|p| call.target.contains(p)) {
            return Decision { allow: false, reason: format!("denied by FLOWR_GATEWAY_DENY ({p})") };
        }
        Decision { allow: true, reason: "policy placeholder: allow".into() }
    }
}

/// AC-15: what the policy says about this call. There's nobody to ask at the gateway, so an
/// `Ask` that reaches it is a refusal.
pub async fn policy_decision(action: flowr_policy::Action, resource: flowr_policy::Resource, call: &Call) -> Decision {
    let Ok(principal) = serde_json::from_value::<flowr_policy::Principal>(call.principal.clone()) else {
        return Decision { allow: false, reason: "the caller's identity can't be read".into() };
    };
    let req = flowr_policy::ExecutionRequest { principal, action, resource, context: Default::default(), attributes: Default::default(), params: Value::Null };
    match flowr_policy::policy_hook().check(&req).await.non_interactive() {
        flowr_policy::PolicyDecision::Allow => Decision { allow: true, reason: "policy: allow".into() },
        flowr_policy::PolicyDecision::Deny { reason, policy_ids } => Decision { allow: false, reason: format!("policy: {reason} ({})", policy_ids.join(", ")) },
        flowr_policy::PolicyDecision::Ask { .. } => Decision { allow: false, reason: "policy: this needs an approval the gateway can't ask for".into() },
    }
}

/// AC-15: the gateway pool of the caller's project in this environment (its environment row,
/// read at most every 30 s per project), else the shared `gateway` pool.
async fn gateway_pool_for(storage: &dyn StorageBackend, project: &str) -> String {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static CACHE: Mutex<Option<HashMap<String, (Instant, String)>>> = Mutex::new(None);
    if let Some((at, pool)) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(project)) {
        if at.elapsed() < Duration::from_secs(30) {
            return pool.clone();
        }
    }
    let env = crate::agent_space::environment();
    let pool = crate::agent_space::environments::of_project(storage, project)
        .await
        .ok()
        .and_then(|rows| rows.into_iter().find(|r| r.get("name").and_then(|n| n.as_str()) == Some(env.as_str())))
        .and_then(|r| r.get("gateway_pool").and_then(|p| p.as_str()).map(String::from))
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| GATEWAY_POOL.to_string());
    let mut cache = CACHE.lock().unwrap();
    cache.get_or_insert_with(HashMap::new).insert(project.to_string(), (Instant::now(), pool.clone()));
    pool
}

/// One call as the gateway sees it.
#[derive(Debug, Clone)]
pub struct Call {
    pub task_type: String,
    /// What is called: the URL, `mcp:<server>/<tool>`, `llm:<resource>`.
    pub target: String,
    pub caller_run: String,
    pub caller_step: Option<String>,
    pub namespace: String,
    pub flow: String,
    pub principal: Value,
}

fn target_of(task_type: &str, task: &NamedTask) -> String {
    let with = match &task.task {
        swf_parser::Task::Call(c) => c.with.clone().unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let s = |k: &str| with.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    match task_type {
        "call:http" => {
            let ep = with.get("endpoint");
            ep.and_then(|e| e.as_str().map(String::from).or_else(|| e.get("uri").and_then(|u| u.as_str()).map(String::from)))
                .unwrap_or_default()
        }
        "call:mcp" => format!("mcp:{}/{}", s("server").if_empty(s("resource")), s("tool")),
        "call:llm" => format!("llm:{}", s("resource").if_empty(s("model"))),
        other => other.to_string(),
    }
}

trait IfEmpty {
    fn if_empty(self, other: String) -> String;
}
impl IfEmpty for String {
    fn if_empty(self, other: String) -> String {
        if self.is_empty() {
            other
        } else {
            self
        }
    }
}

/// Where the call goes: today always A3T's own gateway (this process). An org
/// assigned an external gateway (`orgs.settings.gateway.kind = external`) is G-later.
fn resolve(_call: &Call) -> &'static str {
    "a3t"
}

/// `call:a3t:gateway`: the gateway flow's one step.
pub struct GatewayStep;

#[async_trait]
impl TaskHandler for GatewayStep {
    fn task_type(&self) -> &str {
        GATEWAY_STEP
    }

    async fn execute(&self, task: &NamedTask, input: Value, ctx: &mut ExecutionContext) -> Result<TaskResult> {
        let fail = |m: String| RuntimeError::task_failed(&task.name, m);
        // Verify (placeholder: the caller must be named; identity tokens come with AC-11).
        let caller = input.get("caller").cloned().unwrap_or(Value::Null);
        let task_type = input.get("task_type").and_then(|t| t.as_str()).unwrap_or_default().to_string();
        let call_task: NamedTask = serde_json::from_value(input.get("task").cloned().unwrap_or(Value::Null))
            .map_err(|e| fail(format!("gateway: bad request: {e}")))?;
        let caller_run = caller.get("run_id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        if caller_run.is_empty() {
            return Err(fail("gateway: the request names no caller".into()));
        }
        let call = Call {
            target: target_of(&task_type, &call_task),
            task_type: task_type.clone(),
            caller_run,
            caller_step: caller.get("step").and_then(|v| v.as_str()).map(String::from),
            namespace: caller.get("namespace").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            flow: caller.get("flow").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            principal: caller.get("principal").cloned().unwrap_or(Value::Null),
        };
        // Resolve.
        let via = resolve(&call);
        // Decide.
        let mut decision = PlaceholderPolicy.decide(&call);
        // AC-15: then the policy, on the real target, as the caller.
        if decision.allow {
            if let Some(h) = real_handler(&task_type) {
                decision = policy_decision(h.policy_action(), h.policy_resource(&call_task), &call).await;
            }
        }
        let t0 = Instant::now();
        let outcome: std::result::Result<Value, String> = if !decision.allow {
            Err(format!("denied by the gateway: {}", decision.reason))
        } else {
            // Act, as the caller.
            match real_handler(&task_type) {
                None => Err(format!("gateway: {task_type} is not an outbound call it handles")),
                Some(h) => {
                    let saved = (ctx.namespace.clone(), ctx.principal.clone());
                    ctx.namespace = call.namespace.clone();
                    if let Ok(p) = serde_json::from_value(call.principal.clone()) {
                        ctx.principal = p;
                    }
                    let r = h.execute(&call_task, input.get("input").cloned().unwrap_or(Value::Null), ctx).await;
                    (ctx.namespace, ctx.principal) = saved;
                    r.map(|t| t.output).map_err(|e| e.to_string())
                }
            }
        };
        let took = t0.elapsed();
        info!(
            target = %call.target, task_type = %call.task_type, caller_run = %call.caller_run, namespace = %call.namespace,
            via, allow = decision.allow, reason = %decision.reason, ok = outcome.is_ok(), took_ms = took.as_millis() as u64,
            "gateway decision"
        );
        // Record.
        if let Some(storage) = storage_of(ctx) {
            record(storage.as_ref(), &ctx.instance_id, &call, via, &decision, &outcome, took).await;
        }
        outcome.map(TaskResult::new).map_err(fail)
    }
}

/// A `gateway_decisions` row in the platform database (best effort: a failed write
/// is logged, never fails the call — the gateway run's journal holds the same facts).
async fn record(storage: &dyn StorageBackend, gateway_run: &str, call: &Call, via: &str, d: &Decision, outcome: &std::result::Result<Value, String>, took: Duration) {
    use crate::handlers::call_lancedb::{DEFAULT_PLATFORM_DB_PATH, PLATFORM_DB_RESOURCE};
    let db = crate::platform_db::open_platform_db(storage, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let principal = call.principal.get("id").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| call.principal.to_string());
    let row = json!({
        "id": gateway_run,
        "at": chrono::Utc::now().to_rfc3339(),
        "namespace": call.namespace,
        "flow": call.flow,
        "caller_run": call.caller_run,
        "caller_step": call.caller_step.clone().unwrap_or_default(),
        "principal": principal,
        "task_type": call.task_type,
        "target": call.target,
        "via": via,
        "decision": if d.allow { "allow" } else { "deny" },
        "reason": d.reason,
        "outcome": match outcome { Ok(_) => "ok".to_string(), Err(e) => format!("error: {e}") },
        "took_ms": took.as_millis().to_string(),
        // AC-20. The agent version and memory point are on the caller's `agent_runs` row
        // (`caller_run`).
        "environment": crate::agent_space::environment(),
        "memory_branch": serde_json::from_value::<flowr_policy::Principal>(call.principal.clone()).ok().and_then(|p| p.memory_branch().map(String::from)).unwrap_or_default(),
    });
    if let Err(first) = db.insert("gateway_decisions", std::slice::from_ref(&row)).await {
        // A table made before these columns existed refuses them: keep the decision.
        let mut older = row.clone();
        if let Some(m) = older.as_object_mut() {
            m.remove("environment");
            m.remove("memory_branch");
        }
        if let Err(e) = db.insert("gateway_decisions", &[older]).await {
            warn!(error = %e, first = %first, "gateway: recording the decision failed");
        }
    }
}

/// The gateway flow, seeded into the registry by gateway processes.
pub const GATEWAY_FLOW_YAML: &str = r#"document:
  dsl: '1.0.3'
  namespace: system-gateway
  name: call
  version: '1.0.0'
  summary: One outbound call through the A3T gateway (verify, resolve, decide, act, record).
do:
  - call:
      call: a3t:gateway
      # A gateway that dies mid-call is replaced by another, which runs the call
      # again: HTTP carries the caller step's Idempotency-Key, so the receiver can
      # drop the repeat (MCP and LLM calls have no such key yet).
      metadata: { a3t: { stepClass: idempotent, inDoubt: retry } }
"#;

/// Make sure `system-gateway/call` is in the registry (gateway processes, at start).
pub async fn seed(storage: &dyn StorageBackend) {
    match storage.workflows().get(GATEWAY_NAMESPACE, GATEWAY_FLOW).await {
        Ok(r) if r.raw_yaml == GATEWAY_FLOW_YAML => {}
        _ => match flowr_types::WorkflowRecord::from_yaml(GATEWAY_FLOW_YAML) {
            Ok(rec) => match storage.workflows().register(rec).await {
                Ok(()) => info!("gateway flow system-gateway/call registered"),
                Err(e) => warn!(error = %e, "gateway: registering system-gateway/call failed"),
            },
            Err(e) => warn!(error = %e, "gateway: the built-in flow does not parse"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(target: &str) -> Call {
        Call {
            task_type: "call:http".into(),
            target: target.into(),
            caller_run: "r".into(),
            caller_step: None,
            namespace: "acme".into(),
            flow: "f".into(),
            principal: Value::Null,
        }
    }

    #[test]
    fn the_placeholder_allows_unless_a_deny_pattern_matches() {
        std::env::remove_var("FLOWR_GATEWAY_DENY");
        assert!(PlaceholderPolicy.decide(&call("https://api.example.com/x")).allow);
        std::env::set_var("FLOWR_GATEWAY_DENY", "evil.example, other");
        let d = PlaceholderPolicy.decide(&call("https://evil.example/x"));
        assert!(!d.allow && d.reason.contains("evil.example"));
        assert!(PlaceholderPolicy.decide(&call("https://api.example.com/x")).allow);
        std::env::remove_var("FLOWR_GATEWAY_DENY");
    }

    /// AC-7 in the gateway pool: the branch reaches Decide with the caller's principal.
    #[test]
    fn the_gateway_denies_a_tool_call_from_a_branch_and_allows_its_model() {
        let p = flowr_policy::Principal::human("alice", vec![]).on_branch("try").unwrap();
        let mut c = call("https://api.example.com/x");
        c.principal = serde_json::to_value(&p).unwrap();
        c.task_type = "call:http".into();
        let d = PlaceholderPolicy.decide(&c);
        assert!(!d.allow && d.reason.contains("try"), "{d:?}");
        c.task_type = "call:llm".into();
        assert!(PlaceholderPolicy.decide(&c).allow);
    }

    #[test]
    fn the_gateway_flow_parses() {
        let rec = flowr_types::WorkflowRecord::from_yaml(GATEWAY_FLOW_YAML).expect("parses");
        assert_eq!((rec.namespace(), rec.name()), (GATEWAY_NAMESPACE, GATEWAY_FLOW));
    }
}
