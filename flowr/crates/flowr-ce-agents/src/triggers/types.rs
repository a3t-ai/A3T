//! Trigger types — first-class resources that route events to agents or workflows

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A trigger resource stored in LanceDB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    /// Unique trigger ID (auto-generated if empty)
    #[serde(default)]
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Namespace
    #[serde(default = "default_ns")]
    pub namespace: String,
    /// Trigger configuration (webhook/cron/event)
    pub config: TriggerConfig,
    /// Where to deliver (agents, workflows, webhooks)
    #[serde(default)]
    pub targets: Vec<TriggerTarget>,
    /// Whether the trigger is active
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Current status
    #[serde(default)]
    pub status: TriggerStatus,
    /// Optional description
    #[serde(default)]
    pub description: Option<String>,
    /// AC-12: the environment this trigger belongs to; it fires only in processes serving
    /// it (`A3T_ENVIRONMENT`, default `prod`). `None`: every environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
}

/// Shortest webhook secret accepted (bytes).
pub const MIN_WEBHOOK_SECRET_LEN: usize = 16;

impl Trigger {
    /// A webhook trigger's shared secret, if it has one.
    pub fn webhook_secret(&self) -> Option<&str> {
        match &self.config {
            TriggerConfig::Webhook { secret: Some(s), .. } if !s.is_empty() => Some(s),
            _ => None,
        }
    }

    /// Whether it fires in this process: enabled, and of this process's environment.
    pub fn active_here(&self) -> bool {
        self.enabled && self.environment.as_deref().is_none_or(|e| e == current_environment())
    }
}

fn current_environment() -> String {
    std::env::var("A3T_ENVIRONMENT").ok().filter(|e| !e.is_empty()).unwrap_or_else(|| "prod".to_string())
}

fn default_ns() -> String { "default".to_string() }
fn default_true() -> bool { true }

/// What fires the trigger
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TriggerConfig {
    /// HTTP webhook — fires when POST /webhook?path=<path> is called
    Webhook {
        path: String,
        #[serde(default)]
        method: Option<String>,
        #[serde(default)]
        secret: Option<String>,
        #[serde(default)]
        message_template: Option<String>,
    },
    /// Cron schedule — fires at specified intervals
    Cron {
        expression: String,
        #[serde(default)]
        timezone: Option<String>,
        message: String,
    },
    /// ADB topic — fires once per message published to `topic` on the ADB resource
    /// `resource` (ADB stage F, F5). FlowR holds its own subscription (kept here once
    /// made, so a restart resumes), so every FlowR instance with this trigger sees
    /// every message.
    #[serde(rename = "adb_topic")]
    AdbTopic {
        /// ADB resource (`namespace/name`, or `name` in `default`)
        resource: String,
        topic: String,
        /// Field = value conditions on the message (optional)
        #[serde(default)]
        filter: Option<Value>,
        #[serde(default)]
        message_template: Option<String>,
        /// The ADB subscription id, set when FlowR first subscribes
        #[serde(default)]
        subscription: Option<String>,
    },
    /// Internal event — fires when matching events are emitted
    Event {
        source: String,
        #[serde(default)]
        filter: Option<String>,
        #[serde(default)]
        message: Option<String>,
    },
}

/// Where the trigger delivers its payload
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TriggerTarget {
    /// Deliver to a spawned agent's inbox
    Agent {
        agent_id: String,
    },
    /// Execute a workflow directly (no agent, no LLM cost)
    Workflow {
        namespace: String,
        name: String,
        /// A jq expression over the event — `{body, message, source, trigger, time}`
        /// — whose result is the flow's input. None: the event's body as before.
        #[serde(default)]
        input_mapping: Option<String>,
    },
    /// Send a message to an A3T agent (a turn of `a3t/tmpl_a3t_agent`, in the
    /// conversation `<agent_id>:<conversation>`). `message` and `conversation` are
    /// jq expressions over the event; by default the trigger's message text and
    /// `trigger-<trigger name>`.
    #[serde(rename = "a3t_agent")]
    A3tAgent {
        agent_id: String,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        conversation: Option<String>,
    },
    /// Forward to an external webhook URL
    Webhook {
        url: String,
        #[serde(default)]
        headers: Option<Value>,
    },
}

/// Trigger runtime status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TriggerStatus {
    Active,
    Paused,
    Error(String),
}

impl Default for TriggerStatus {
    fn default() -> Self {
        Self::Active
    }
}

/// Result of a trigger delivery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerDeliveryResult {
    pub trigger_id: String,
    pub trigger_name: String,
    pub delivered_to: Vec<String>,
    pub failed: Vec<String>,
    /// The runs this firing started (a flow or an A3T agent's turn), so a caller
    /// can follow each one.
    #[serde(default)]
    pub executions: Vec<TriggerExecution>,
}

/// A run a trigger started: which target, and its instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerExecution {
    pub target: String,
    pub instance_id: String,
}

