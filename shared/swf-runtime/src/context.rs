//! Execution context — runtime expression arguments and task state

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Workflow-level execution context
pub struct ExecutionContext {
    /// Workflow instance ID
    pub instance_id: String,

    /// Workflow namespace
    pub namespace: String,

    /// Workflow name
    pub name: String,

    /// Workflow version
    pub version: String,

    /// Mutable workflow context ($context)
    pub context: Value,

    /// Secrets ($secrets)
    pub secrets: HashMap<String, String>,

    /// Runtime metadata ($runtime)
    pub runtime: RuntimeInfo,

    /// Storage backend (trait object for resource/state access)
    pub storage: Option<Arc<dyn std::any::Any + Send + Sync>>,

    /// HTTP client for call:http tasks
    pub http_client: Option<Arc<dyn std::any::Any + Send + Sync>>,

    /// Resolved extensions from workflow's use.extensions block
    pub extensions: Vec<crate::executor::extensions::ResolvedExtension>,

    /// Expression evaluator function (needed for extension when-conditions and tasks)
    pub eval_fn: Option<Arc<dyn Fn(&Value, &str, &Value) -> std::result::Result<Value, String> + Send + Sync>>,

    /// E0.4 — the security principal this workflow runs as. Set by whoever
    /// starts the flow (a run_workflow handler, an agent spawn) so that choke
    /// point B can authorize against the real actor instead of an anonymous
    /// runtime. Defaults to [`default_flow_principal`] (a `Service` identity)
    /// when the caller has no better identity to supply.
    pub principal: flowr_policy::Principal,

    /// E2 #2a — the flow's `metadata.a3t` template data, wrapped as
    /// `{"template": {...}}` (exposedTools, runtimePolicy, lockedFields, …). The
    /// choke point copies it into the `ExecutionRequest.attributes` so
    /// InfoSec-owned policies can gate on `principal.template.*`. Empty when the
    /// flow declares no `metadata.a3t`.
    pub template_meta: serde_json::Map<String, Value>,

    /// AC-8: steps this run has taken and may take, shared with its subflows.
    pub limits: RunLimits,
}

/// AC-8: a run's step budget. A looping flow stops here instead of holding a worker. The most
/// is `FLOWR_RUN_MAX_STEPS` (default 100,000); a flow can ask for less with
/// `metadata.a3t.limits.max_steps`, never more.
#[derive(Debug, Clone)]
pub struct RunLimits {
    steps: std::sync::Arc<std::sync::atomic::AtomicU64>,
    max: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Default for RunLimits {
    fn default() -> Self {
        let max = std::env::var("FLOWR_RUN_MAX_STEPS").ok().and_then(|v| v.parse().ok()).unwrap_or(100_000);
        Self { steps: Default::default(), max: std::sync::Arc::new(max.into()) }
    }
}

impl RunLimits {
    /// Lower the budget (attenuate only).
    pub fn narrow(&self, max: u64) {
        self.max.fetch_min(max, std::sync::atomic::Ordering::SeqCst);
    }

    /// Count a step; `Err(limit)` when it passes the budget.
    pub fn step(&self) -> std::result::Result<(), u64> {
        use std::sync::atomic::Ordering::SeqCst;
        let (taken, max) = (self.steps.fetch_add(1, SeqCst) + 1, self.max.load(SeqCst));
        if taken > max { Err(max) } else { Ok(()) }
    }
}

/// The fallback principal for a flow with no caller-supplied identity: the
/// runtime service itself. Threading a real `Human`/`Agent` principal is the
/// job of the flow-start call site; until it does, this preserves today's
/// behavior (choke point B sees `Service{"flow-runtime"}`).
pub fn default_flow_principal() -> flowr_policy::Principal {
    flowr_policy::Principal::Service { service_id: "flow-runtime".into(), memory_branch: None }
}

/// Runtime information ($runtime argument)
#[derive(Debug, Clone)]
pub struct RuntimeInfo {
    pub name: String,
    pub version: String,
    pub metadata: HashMap<String, Value>,
}

impl Default for RuntimeInfo {
    fn default() -> Self {
        Self {
            name: "clawdflow".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            metadata: HashMap::new(),
        }
    }
}

/// Task-level execution context
pub struct TaskContext {
    /// Task name
    pub name: String,

    /// Task reference (JSON Pointer path, e.g. "/do/fetchData")
    pub reference: String,

    /// Task input (after input.from transform)
    pub input: Value,

    /// Current workflow context
    pub context: Value,

    /// Task output (set after execution)
    pub output: Option<Value>,
}

impl TaskContext {
    pub fn new(name: String, reference: String, input: Value, context: Value) -> Self {
        Self {
            name,
            reference,
            input,
            context,
            output: None,
        }
    }

    /// Build the $task runtime expression argument
    pub fn task_descriptor(&self) -> Value {
        serde_json::json!({
            "name": self.name,
            "reference": self.reference,
            "input": self.input,
        })
    }
}
