//! The decision core (was the AESOP `aesop-core` crate): the shared vocabulary
//! (Principal / Action / Resource / SecurityContext / Decision), the engine trait
//! and the Cedar-backed `AuthzEngine`.
//!
//! Invariants (normative — see .ai/AESOP_SPEC.md):
//! - Deny-by-default: no matching policy ⇒ Deny.
//! - Denials are never silent: every Deny is an audit event.
//! - EntityUids are never assembled by string formatting.
//! - One enforcement point per request path.

use serde::{Deserialize, Serialize};

/// Stable identity of an agent instance. Stable across a run; attributable
/// across runs of the same configured agent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

/// Who (or what) is acting. Mirrors the A3T CE contract §5.2 — CE constructs
/// these; AESOP consumes them unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Principal {
    Human {
        user_id: String,
        roles: Vec<String>,
    },
    Agent {
        agent_id: AgentId,
        /// The human or tenant this agent acts for.
        on_behalf_of: Option<String>,
        flow_id: Option<String>,
        /// Authority inherited from the invoking principal, already attenuated
        /// by the caller. Policy reads this exactly as it reads `User.roles`, so
        /// an agent acting for a Viewer is distinguishable from one acting for
        /// an Admin — previously it was not, and both got the same blanket
        /// permit on the strength of `on_behalf_of` alone.
        #[serde(default)]
        roles: Vec<String>,
        /// The principal that owns this agent. Bounds what it may be granted.
        #[serde(default)]
        owner: Option<String>,
    },
    Service {
        service_id: String,
    },
}

impl Principal {
    /// Telemetry discriminant (`aesop.principal.kind`).
    pub fn kind(&self) -> &'static str {
        match self {
            Principal::Human { .. } => "human",
            Principal::Agent { .. } => "agent",
            Principal::Service { .. } => "service",
        }
    }
    /// Telemetry id (`aesop.principal.id`).
    pub fn id(&self) -> &str {
        match self {
            Principal::Human { user_id, .. } => user_id,
            Principal::Agent { agent_id, .. } => &agent_id.0,
            Principal::Service { service_id } => service_id,
        }
    }
}

/// Typed, namespaced action: `"{domain}:{verb}"` — e.g. `tool:invoke`,
/// `flow:start`, `mcp:call`, `media_buy:create`. Never a free-form string
/// assembled at a call site; construct via [`Action::new`] which validates
/// the shape.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Action(String);

impl Action {
    /// Validates `domain:verb` shape. Additions to the vocabulary are
    /// reviewed changes (CE contract §5.3).
    pub fn new(s: impl Into<String>) -> Result<Self, AesopError> {
        let s = s.into();
        let mut parts = s.splitn(2, ':');
        match (parts.next(), parts.next()) {
            (Some(d), Some(v)) if !d.is_empty() && !v.is_empty() => Ok(Self(s)),
            _ => Err(AesopError::InvalidAction(s)),
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A typed resource: type name + id (+ attributes the policy may reference).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    /// Cedar entity type name, e.g. "Tool", "Flow", "Table".
    pub type_name: String,
    pub id: String,
    /// Attributes the schema declares (e.g. `destructive: true`).
    #[serde(default)]
    pub attributes: serde_json::Map<String, serde_json::Value>,
    /// Entity tags (a Tool's whole-number call arguments, for `getTag`).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub tags: std::collections::BTreeMap<String, i64>,
}

/// Ambient security context threaded from the edge (authn layer) to the
/// enforcement point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityContext {
    pub principal: Principal,
    #[serde(default)]
    pub attributes: serde_json::Map<String, serde_json::Value>,
    /// The Cedar request context (`context.*`): Cedar JSON, extension values as
    /// `{"__extn": {"fn": "ip", "arg": "10.0.0.1"}}`. Checked against the
    /// action's declared context; empty for an action that declares none.
    #[serde(default)]
    pub context: serde_json::Map<String, serde_json::Value>,
}

/// The outcome of an authorization decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Decision {
    Allow,
    Deny {
        reason: String,
        policy_ids: Vec<String>,
    },
}

/// Proof-of-allow returned by `enforce()`. Engine faults and denials are both
/// errors so an allow can never be fabricated from a fault path.
#[derive(Debug, Clone)]
pub struct Allowed {
    pub policy_ids: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AesopError {
    #[error("denied: {action} on {resource_type}::{resource_id}")]
    Denied {
        action: String,
        resource_type: String,
        resource_id: String,
        policy_ids: Vec<String>,
    },
    #[error("invalid action name (want domain:verb): {0}")]
    InvalidAction(String),
    #[error("engine fault: {0}")]
    Engine(String),
}

/// The engine surface. The Cedar implementation arrives in Phase 1 behind the
/// `cedar` feature; `PermissiveEngine` exists so CE-shaped consumers and tests
/// can exercise the full call path (including Deny, via `DenyAllEngine`).
pub trait Engine: Send + Sync {
    fn enforce(
        &self,
        ctx: &SecurityContext,
        action: &Action,
        resource: &Resource,
    ) -> Result<Allowed, AesopError>;
}

/// Allows everything. The semantic twin of A3T CE's `AllowAll` PolicyHook.
pub struct PermissiveEngine;

impl Engine for PermissiveEngine {
    fn enforce(
        &self,
        _ctx: &SecurityContext,
        _action: &Action,
        _resource: &Resource,
    ) -> Result<Allowed, AesopError> {
        Ok(Allowed { policy_ids: vec![] })
    }
}

/// Denies everything. Exists so consumers can test their Deny semantics
/// (CE contract §5.2: the denied path is handled and tested in CE).
pub struct DenyAllEngine;

impl Engine for DenyAllEngine {
    fn enforce(
        &self,
        _ctx: &SecurityContext,
        action: &Action,
        resource: &Resource,
    ) -> Result<Allowed, AesopError> {
        Err(AesopError::Denied {
            action: action.as_str().to_string(),
            resource_type: resource.type_name.clone(),
            resource_id: resource.id.clone(),
            policy_ids: vec!["deny-all".into()],
        })
    }
}

// Phase 1: the Cedar-backed engine (feature `cedar`). Wraps
// cedar_policy::Authorizer + PolicySet + Schema; EntityUids built only via
// from_type_name_and_id (N6); deny-by-default (N4); schema-validated (N8).
mod cedar_engine;
pub use cedar_engine::AuthzEngine;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_shape_is_validated() {
        assert!(Action::new("tool:invoke").is_ok());
        assert!(Action::new("media_buy:create").is_ok());
        assert!(Action::new("toolinvoke").is_err());
        assert!(Action::new(":x").is_err());
        assert!(Action::new("x:").is_err());
    }

    #[test]
    fn deny_all_produces_auditable_denial() {
        let ctx = SecurityContext {
            principal: Principal::Agent {
                agent_id: AgentId("pc-crawler-7".into()),
                on_behalf_of: Some("user::sriram".into()),
                flow_id: Some("flow-1".into()),
                roles: vec![],
                owner: None,
            },
            attributes: Default::default(),
            context: Default::default(),
        };
        let action = Action::new("tool:invoke").unwrap();
        let resource = Resource {
            type_name: "Tool".into(),
            id: "http_fetch".into(),
            attributes: Default::default(),
            tags: Default::default(),
        };
        let err = DenyAllEngine.enforce(&ctx, &action, &resource).unwrap_err();
        match err {
            AesopError::Denied { policy_ids, .. } => assert_eq!(policy_ids, vec!["deny-all"]),
            other => panic!("expected Denied, got {other:?}"),
        }
    }
}
