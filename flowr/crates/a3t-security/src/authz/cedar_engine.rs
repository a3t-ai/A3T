//! `AuthzEngine` — the Cedar-backed decision engine (Phase 1, feature `cedar`).
//!
//! Wraps `cedar_policy` behind the [`super::Engine`] trait so consumers see the
//! same `enforce()` surface as `PermissiveEngine`/`DenyAllEngine`. Normative
//! rules enforced here (see `.ai/AESOP_SPEC.md`):
//! - **N4 deny-by-default** — Cedar denies unless a `permit` matches.
//! - **N5 denials are never silent** — every `Deny` emits a `tracing` audit event.
//! - **N6 no string-built EntityUids** — [`uid`] uses `from_type_name_and_id`.
//! - **N8 schema-validated policies** — a supplied schema validates the policy
//!   set at load; invalid policies are rejected (cannot construct the engine).

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use cedar_policy::{
    Authorizer, Context, Decision as CedarDecision, Entities, Entity, EntityId, EntityTypeName,
    EntityUid, PolicySet, Request, RestrictedExpression, Schema, ValidationMode, Validator,
};

use super::{AesopError, Action, Allowed, Engine, Principal, Resource, SecurityContext};

/// Production engine: a Cedar policy set + optional schema.
pub struct AuthzEngine {
    authorizer: Authorizer,
    policies: PolicySet,
    schema: Option<Schema>,
}

impl AuthzEngine {
    /// Build from Cedar policy source and an optional cedarschema. With a
    /// schema, the policy set is validated (N8) — invalid policies fail here.
    pub fn new(policies_src: &str, schema_src: Option<&str>) -> Result<Self, AesopError> {
        let policies = PolicySet::from_str(policies_src)
            .map_err(|e| AesopError::Engine(format!("policy parse: {e}")))?;

        let schema = match schema_src {
            Some(s) => {
                // Accept either a cedarschema (human syntax) or a Cedar JSON schema.
                // The registry/merge pipeline produces JSON (starts with '{'); the
                // embedded default and `from_dir` use cedarschema.
                let schema = if s.trim_start().starts_with('{') {
                    Schema::from_json_str(s)
                        .map_err(|e| AesopError::Engine(format!("json schema parse: {e}")))?
                } else {
                    let (schema, _warnings) = Schema::from_cedarschema_str(s)
                        .map_err(|e| AesopError::Engine(format!("schema parse: {e}")))?;
                    schema
                };
                // N8: reject a policy set that does not validate against the schema.
                let validator = Validator::new(schema.clone());
                let result = validator.validate(&policies, ValidationMode::default());
                if !result.validation_passed() {
                    let errs: Vec<String> =
                        result.validation_errors().map(|e| e.to_string()).collect();
                    return Err(AesopError::Engine(format!(
                        "schema validation failed: {}",
                        errs.join("; ")
                    )));
                }
                Some(schema)
            }
            None => None,
        };

        Ok(Self { authorizer: Authorizer::new(), policies, schema })
    }

    /// Parse-check a schema source (JSON or cedarschema) in isolation — used to
    /// validate a per-resource schema fragment before merging it, so one bad
    /// fragment is skipped rather than breaking the whole schema.
    pub fn schema_parses(schema_src: &str) -> bool {
        if schema_src.trim_start().starts_with('{') {
            Schema::from_json_str(schema_src).is_ok()
        } else {
            Schema::from_cedarschema_str(schema_src).is_ok()
        }
    }
}

/// N6: the only place an `EntityUid` is constructed — via
/// `from_type_name_and_id`, never string-formatted. (A CI grep lint forbids
/// `format!(...::"...")` elsewhere.)
fn uid(type_name: &str, id: &str) -> Result<EntityUid, AesopError> {
    let tn = EntityTypeName::from_str(type_name)
        .map_err(|e| AesopError::Engine(format!("bad entity type {type_name}: {e}")))?;
    let eid = EntityId::from_str(id)
        .map_err(|e| AesopError::Engine(format!("bad entity id {id}: {e}")))?;
    Ok(EntityUid::from_type_name_and_id(tn, eid))
}

/// Cedar entity type + id for a principal (its attributes come from [`principal_attrs`]).
fn principal_uid(p: &Principal) -> Result<EntityUid, AesopError> {
    match p {
        Principal::Human { user_id, .. } => uid("User", user_id),
        Principal::Agent { agent_id, .. } => uid("Agent", &agent_id.0),
        Principal::Service { service_id } => uid("Service", service_id),
    }
}

fn json_to_restricted(v: &serde_json::Value) -> Result<RestrictedExpression, AesopError> {
    use serde_json::Value;
    Ok(match v {
        Value::Bool(b) => RestrictedExpression::new_bool(*b),
        Value::String(s) => RestrictedExpression::new_string(s.clone()),
        Value::Number(n) if n.is_i64() => RestrictedExpression::new_long(n.as_i64().unwrap()),
        // Sets (e.g. `exposedTools: ["http","query"]`) and nested records
        // (e.g. `template: { exposedTools: [...], runtimePolicy: "sandboxed" }`)
        // so policies can gate on `principal.template.exposedTools` etc.
        Value::Array(arr) => {
            let items = arr
                .iter()
                .map(json_to_restricted)
                .collect::<Result<Vec<_>, _>>()?;
            RestrictedExpression::new_set(items)
        }
        Value::Object(map) => {
            let fields = map
                .iter()
                .map(|(k, v)| Ok::<_, AesopError>((k.clone(), json_to_restricted(v)?)))
                .collect::<Result<Vec<_>, _>>()?;
            RestrictedExpression::new_record(fields)
                .map_err(|e| AesopError::Engine(format!("record attr: {e}")))?
        }
        other => {
            return Err(AesopError::Engine(format!(
                "unsupported attribute value: {other}"
            )))
        }
    })
}

fn attrs_to_expr(
    attrs: &serde_json::Map<String, serde_json::Value>,
) -> Result<HashMap<String, RestrictedExpression>, AesopError> {
    attrs
        .iter()
        .map(|(k, v)| Ok((k.clone(), json_to_restricted(v)?)))
        .collect()
}

/// Principal attributes the policy may read (`principal.roles`, `.on_behalf_of`, …).
fn principal_attrs(
    p: &Principal,
) -> Result<HashMap<String, RestrictedExpression>, AesopError> {
    let mut attrs = HashMap::new();
    match p {
        Principal::Human { roles, .. } => {
            let list = roles
                .iter()
                .map(|r| RestrictedExpression::new_string(r.clone()))
                .collect::<Vec<_>>();
            attrs.insert("roles".to_string(), RestrictedExpression::new_set(list));
        }
        Principal::Agent { on_behalf_of, roles, owner, .. } => {
            if let Some(obo) = on_behalf_of {
                attrs.insert(
                    "on_behalf_of".to_string(),
                    RestrictedExpression::new_string(obo.clone()),
                );
            }
            // The agent's inherited, already-attenuated authority. Emitted under
            // the same attribute name as a User's, so one rule can cover both
            // (`principal has roles && principal.roles.contains("Admin")`)
            // instead of needing a parallel Agent-only variant.
            let list = roles
                .iter()
                .map(|r| RestrictedExpression::new_string(r.clone()))
                .collect::<Vec<_>>();
            attrs.insert("roles".to_string(), RestrictedExpression::new_set(list));
            if let Some(o) = owner {
                attrs.insert(
                    "owner".to_string(),
                    RestrictedExpression::new_string(o.clone()),
                );
            }
        }
        Principal::Service { .. } => {}
    }
    Ok(attrs)
}

fn reason_ids(resp: &cedar_policy::Response) -> Vec<String> {
    resp.diagnostics().reason().map(|p| p.to_string()).collect()
}

impl Engine for AuthzEngine {
    fn enforce(
        &self,
        ctx: &SecurityContext,
        action: &Action,
        resource: &Resource,
    ) -> Result<Allowed, AesopError> {
        let principal_uid = principal_uid(&ctx.principal)?;
        let action_uid = uid("Action", action.as_str())?;
        let resource_uid = uid(&resource.type_name, &resource.id)?;

        // Entity store: the principal and resource carry the attributes policies
        // read. Principal attributes come from the Principal itself (roles /
        // on_behalf_of) plus the ambient SecurityContext.attributes — the latter
        // is where the acting agent's template data lands (`template.exposedTools`,
        // `runtimePolicy`, …), so resource-referenced rules can gate on it.
        let mut p_attrs = principal_attrs(&ctx.principal)?;
        for (k, v) in &ctx.attributes {
            p_attrs.insert(k.clone(), json_to_restricted(v)?);
        }
        let principal_entity = Entity::new(principal_uid.clone(), p_attrs, HashSet::new())
            .map_err(|e| AesopError::Engine(format!("principal entity: {e}")))?;
        let resource_entity = Entity::new(
            resource_uid.clone(),
            attrs_to_expr(&resource.attributes)?,
            HashSet::new(),
        )
        .map_err(|e| AesopError::Engine(format!("resource entity: {e}")))?;

        // Build the entity store WITHOUT schema validation. Principal/resource
        // carry *dynamic, open-ended* attributes the choke points attach — the
        // tool's `name`, and the acting agent's `template.*` data lifted from
        // `metadata.a3t` (whose shape is per-template, §6.3/§7). Validating these
        // against a closed schema would fail-closed on any attribute the schema
        // doesn't enumerate. Policies are still validated against the schema at
        // load (`AuthzEngine::new`), and the Request below is schema-checked for
        // action/principal/resource *type* binding — only the free-form attribute
        // bag is left unvalidated, which is exactly what `has`-guarded rules want.
        let entities = Entities::from_entities([principal_entity, resource_entity], None)
            .map_err(|e| AesopError::Engine(format!("entities: {e}")))?;

        let request = Request::new(
            principal_uid,
            action_uid,
            resource_uid,
            Context::empty(),
            self.schema.as_ref(),
        )
        .map_err(|e| AesopError::Engine(format!("request: {e}")))?;

        let resp = self.authorizer.is_authorized(&request, &self.policies, &entities);
        match resp.decision() {
            CedarDecision::Allow => Ok(Allowed { policy_ids: reason_ids(&resp) }),
            CedarDecision::Deny => {
                let policy_ids = reason_ids(&resp);
                // N5: denials are never silent.
                tracing::warn!(
                    "aesop.decision" = "deny",
                    "aesop.action" = action.as_str(),
                    "aesop.resource.type" = %resource.type_name,
                    "aesop.resource.id" = %resource.id,
                    "aesop.principal.kind" = ctx.principal.kind(),
                    "aesop.principal.id" = %ctx.principal.id(),
                    policies = ?policy_ids,
                    "authorization denied"
                );
                Err(AesopError::Denied {
                    action: action.as_str().to_string(),
                    resource_type: resource.type_name.clone(),
                    resource_id: resource.id.clone(),
                    policy_ids,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AuthzEngine;
    use crate::{Action, AgentId, AesopError, Engine, Principal, Resource, SecurityContext};

    const SCHEMA: &str = r#"
entity User { roles?: Set<String> };
entity Agent { on_behalf_of?: String, roles?: Set<String>, owner?: String };
entity Service;
entity Tool { destructive?: Bool };
action "tool:invoke" appliesTo {
  principal: [User, Agent, Service],
  resource: [Tool]
};
"#;

    // Baseline pack: admins may invoke tools; destructive tools are forbidden for
    // everyone (forbid overrides permit); everything else is deny-by-default.
    const POLICIES: &str = r#"
permit(principal, action == Action::"tool:invoke", resource)
when { principal is User && principal has roles && principal.roles.contains("admin") };

forbid(principal, action == Action::"tool:invoke", resource)
when { resource has destructive && resource.destructive };
"#;

    fn engine() -> AuthzEngine {
        AuthzEngine::new(POLICIES, Some(SCHEMA)).expect("policies validate against schema (N8)")
    }
    fn human(roles: &[&str]) -> SecurityContext {
        SecurityContext {
            principal: Principal::Human {
                user_id: "sriram".into(),
                roles: roles.iter().map(|s| s.to_string()).collect(),
            },
            attributes: Default::default(),
        }
    }
    fn tool(id: &str, destructive: bool) -> Resource {
        let mut attributes = serde_json::Map::new();
        attributes.insert("destructive".into(), serde_json::json!(destructive));
        Resource { type_name: "Tool".into(), id: id.into(), attributes }
    }
    fn invoke() -> Action { Action::new("tool:invoke").unwrap() }

    #[test]
    fn admin_allowed_on_safe_tool() {
        assert!(engine().enforce(&human(&["admin"]), &invoke(), &tool("http_fetch", false)).is_ok());
    }

    #[test]
    fn destructive_tool_forbidden_even_for_admin() {
        let err = engine().enforce(&human(&["admin"]), &invoke(), &tool("db_drop", true)).unwrap_err();
        assert!(matches!(err, AesopError::Denied { .. }), "forbid overrides permit: {err:?}");
    }

    #[test]
    fn non_admin_denied_by_default() {
        // N4: no permit matches a viewer → Deny.
        let err = engine().enforce(&human(&["viewer"]), &invoke(), &tool("http_fetch", false)).unwrap_err();
        assert!(matches!(err, AesopError::Denied { .. }));
    }

    #[test]
    fn agent_denied_by_default() {
        let ctx = SecurityContext {
            principal: Principal::Agent {
                agent_id: AgentId("pc-crawler-7".into()),
                on_behalf_of: Some("user::sriram".into()),
                flow_id: None,
                roles: vec![],
                owner: None,
            },
            attributes: Default::default(),
        };
        let err = engine().enforce(&ctx, &invoke(), &tool("http_fetch", false)).unwrap_err();
        assert!(matches!(err, AesopError::Denied { .. }));
    }

    #[test]
    fn schema_invalid_policy_is_rejected() {
        // N8: a policy referencing an undeclared attribute must not load.
        let bad = r#"permit(principal, action == Action::"tool:invoke", resource) when { resource.nonexistent };"#;
        assert!(AuthzEngine::new(bad, Some(SCHEMA)).is_err(), "schema-invalid policy must not load");
    }

    // E2 #2a — a policy that gates on the acting agent's *template* data
    // (`principal.template.exposedTools`), fed via SecurityContext.attributes.
    // This is the mechanism the InfoSec-owned policy resources rely on: rules
    /// The gap this closes: before agents carried `roles`, `on_behalf_of` told
    /// policy WHOSE name an agent acted under but nothing about what that person
    /// could do — so an agent run by a Viewer and one run by an Admin were
    /// indistinguishable and both matched the same blanket permit.
    #[test]
    fn agent_roles_are_inherited_authority_that_policy_can_read() {
        const SCHEMA: &str = r#"
entity User { roles?: Set<String> };
entity Agent { on_behalf_of?: String, roles?: Set<String>, owner?: String };
entity Service;
entity Tool { destructive?: Bool, name?: String };
action "tool:invoke" appliesTo { principal: [User, Agent, Service], resource: [Tool] };
"#;
        // One rule, written against `principal.roles`, covering User and Agent
        // alike — no Agent-specific variant needed.
        const POLICIES: &str = r#"
permit(principal, action == Action::"tool:invoke", resource)
when { principal has roles && principal.roles.contains("Admin") };
"#;
        let engine = AuthzEngine::new(POLICIES, Some(SCHEMA)).expect("policy validates");
        let agent = |roles: &[&str]| SecurityContext {
            principal: Principal::Agent {
                agent_id: AgentId("agt-1".into()),
                on_behalf_of: Some("usr-sriram".into()),
                flow_id: None,
                roles: roles.iter().map(|r| r.to_string()).collect(),
                owner: Some("usr-sriram".into()),
            },
            attributes: Default::default(),
        };
        let act = Action::new("tool:invoke").unwrap();
        let tool = Resource { type_name: "Tool".into(), id: "http_fetch".into(), attributes: Default::default() };

        // Acting for an Admin: the inherited role satisfies the rule.
        assert!(engine.enforce(&agent(&["Admin"]), &act, &tool).is_ok());

        // Same agent, same tool, acting for a Viewer: denied. Identical
        // `on_behalf_of` shape — only the inherited authority differs, which is
        // exactly what was invisible to policy before.
        let err = engine.enforce(&agent(&["Viewer"]), &act, &tool).unwrap_err();
        assert!(matches!(err, AesopError::Denied { .. }), "expected denial: {err:?}");

        // And an agent carrying no roles at all (service-invoked) is denied.
        let err = engine.enforce(&agent(&[]), &act, &tool).unwrap_err();
        assert!(matches!(err, AesopError::Denied { .. }), "expected denial: {err:?}");
    }

    // central, data per-template.
    #[test]
    fn template_exposed_tools_gates_agent() {
        const SCHEMA: &str = r#"
entity User { roles?: Set<String> };
entity Agent {
  on_behalf_of?: String,
  roles?: Set<String>,
  owner?: String,
  template?: { "exposedTools": Set<String>, "runtimePolicy"?: String }
};
entity Service;
entity Tool { destructive?: Bool, name?: String };
action "tool:invoke" appliesTo { principal: [User, Agent, Service], resource: [Tool] };
"#;
        const POLICIES: &str = r#"
permit(principal is Agent, action == Action::"tool:invoke", resource)
when {
  principal has template && resource has name &&
  principal.template.exposedTools.contains(resource.name)
};
"#;
        let engine = AuthzEngine::new(POLICIES, Some(SCHEMA)).expect("policy validates");

        // Agent whose template exposes only ["http","query"].
        let ctx = SecurityContext {
            principal: Principal::Agent {
                agent_id: AgentId("agt-1".into()),
                on_behalf_of: Some("user::sriram".into()),
                flow_id: None,
                roles: vec![],
                owner: None,
            },
            attributes: serde_json::json!({
                "template": { "exposedTools": ["http", "query"], "runtimePolicy": "sandboxed" }
            })
            .as_object()
            .unwrap()
            .clone(),
        };
        let named = |name: &str| {
            let mut a = serde_json::Map::new();
            a.insert("name".into(), serde_json::json!(name));
            Resource { type_name: "Tool".into(), id: name.into(), attributes: a }
        };
        assert!(
            engine.enforce(&ctx, &invoke(), &named("http")).is_ok(),
            "tool in template.exposedTools is allowed"
        );
        assert!(
            engine.enforce(&ctx, &invoke(), &named("shell")).is_err(),
            "tool NOT in template.exposedTools is denied"
        );
    }
}
