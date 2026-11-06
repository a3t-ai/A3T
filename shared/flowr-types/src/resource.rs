//! Resource types for FlowR Hub

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How the resource manages connections
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    /// Connection pool (databases, redis)
    #[default]
    Pool,
    /// On-demand connections (HTTP APIs)
    OnDemand,
    /// Persistent connection (websockets, streams)
    Persistent,
}

/// Category of resource for UI organization
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResourceCategory {
    /// SQL databases (mysql, postgres, sqlite)
    Database,
    /// Key-value stores (redis, dragonfly)
    KeyValue,
    /// Vector databases (qdrant, pinecone, milvus)
    Vector,
    /// Message queues (kafka, rabbitmq, iggy)
    Queue,
    /// HTTP/REST APIs
    #[default]
    Api,
    /// Object storage (s3, gcs, minio)
    Storage,
    /// LLM providers (Anthropic, OpenAI, Ollama)
    Ai,
    /// AI Agents
    Agent,
    /// MCP servers
    Mcp,
    /// Agent Database memory
    Memory,
    /// Input/Output channels
    Channel,
    /// Secrets and credentials
    Security,
    /// Custom/other
    Custom,
}

/// Resource Type - Template in the Hub
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceType {
    /// Unique identifier (e.g., "mysql", "qdrant", "rest_api")
    pub name: String,

    /// Human-readable description
    pub description: String,

    /// Category for organization
    #[serde(default)]
    pub category: ResourceCategory,

    /// JSON Schema defining required configuration
    pub config_schema: Value,

    /// How connections are managed
    #[serde(default)]
    pub connection_kind: ConnectionKind,

    /// Icon identifier for UI
    #[serde(default)]
    pub icon: Option<String>,

    /// Whether this is a built-in type
    #[serde(default)]
    pub builtin: bool,

    /// Version of the type schema
    #[serde(default = "default_version")]
    pub version: String,
}

fn default_version() -> String {
    "1.0".to_string()
}

impl ResourceType {
    /// Create a new resource type
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            category: ResourceCategory::default(),
            config_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            connection_kind: ConnectionKind::default(),
            icon: None,
            builtin: false,
            version: default_version(),
        }
    }

    /// Set the category
    pub fn with_category(mut self, category: ResourceCategory) -> Self {
        self.category = category;
        self
    }

    /// Set the config schema
    pub fn with_schema(mut self, schema: Value) -> Self {
        self.config_schema = schema;
        self
    }

    /// Mark as builtin
    pub fn as_builtin(mut self) -> Self {
        self.builtin = true;
        self
    }
}

/// Resource - Configured instance of a ResourceType
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    /// Unique name within namespace
    pub name: String,

    /// Namespace for organization
    #[serde(default = "default_namespace")]
    pub namespace: String,

    /// Reference to the resource type
    pub type_ref: String,

    /// Actual configuration values
    pub config: Value,

    /// Optional description
    #[serde(default)]
    pub description: Option<String>,

    /// Tags for filtering
    #[serde(default)]
    pub tags: Vec<String>,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Last update timestamp
    pub updated_at: DateTime<Utc>,

    /// Created by (user or system)
    #[serde(default)]
    pub created_by: Option<String>,
}

fn default_namespace() -> String {
    "default".to_string()
}

impl Resource {
    /// Create a new resource
    pub fn new(
        name: impl Into<String>,
        type_ref: impl Into<String>,
        config: Value,
    ) -> Self {
        let now = Utc::now();
        Self {
            name: name.into(),
            namespace: default_namespace(),
            type_ref: type_ref.into(),
            config,
            description: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            created_by: None,
        }
    }

    /// Set namespace
    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = namespace.into();
        self
    }

    /// Set description
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Get the full resource path (namespace/name)
    pub fn path(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

impl Resource {
    /// AC-13: this resource as this process's environment sees it (`A3T_ENVIRONMENT`, default
    /// `prod`): `config.environments.<env>` laid over the rest of `config`. What a flow is
    /// handed at run time; what's stored and shown keeps every environment's values.
    pub fn in_environment(mut self) -> Self {
        let env = std::env::var("A3T_ENVIRONMENT").ok().filter(|e| !e.is_empty()).unwrap_or_else(|| "prod".to_string());
        self.config = config_in_environment(&self.config, &env);
        self
    }
}

/// `config` with `environments.<env>`'s fields laid over it, and `environments` removed.
pub fn config_in_environment(config: &Value, env: &str) -> Value {
    let Some(obj) = config.as_object() else { return config.clone() };
    let mut out = obj.clone();
    let overlay = out.remove("environments");
    if let Some(fields) = overlay.as_ref().and_then(|o| o.get(env)).and_then(|o| o.as_object()) {
        for (k, v) in fields {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod environment_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_environment_overlays_its_values() {
        let c = json!({ "url": "https://prod", "model": "m", "environments": { "dev": { "url": "https://dev" }, "qa": { "url": "https://qa", "model": "q" } } });
        assert_eq!(config_in_environment(&c, "dev"), json!({ "url": "https://dev", "model": "m" }));
        assert_eq!(config_in_environment(&c, "qa"), json!({ "url": "https://qa", "model": "q" }));
        assert_eq!(config_in_environment(&c, "prod"), json!({ "url": "https://prod", "model": "m" }));
        assert_eq!(config_in_environment(&json!("x"), "dev"), json!("x"));
    }
}
