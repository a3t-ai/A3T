//! The Cedar policy hook (was the `flowr-aesop` glue crate).
//!
//! [`CedarHook`] implements CE's [`flowr_policy::PolicyHook`] by translating a
//! CE [`ExecutionRequest`] into an aesop [`SecurityContext`]/[`Action`]/
//! [`Resource`] and running it through [`crate::AuthzEngine`] (Cedar).
//! This *is* the CE→EE enforcement upgrade: CE ships `AllowAll`; installing
//! this hook via `security.policy_hook = "cedar"` turns policy into enforcement,
//! with **no CE code change**.
//!
//! Dependency direction: `flowr-server (ee) → a3t-security → flowr-policy`. Neither
//! CE nor aesop depends on this crate.


use crate::authz::{
    AesopError, Action as AesopAction, AgentId as AesopAgentId, AuthzEngine, Engine,
    Principal as AesopPrincipal, Resource as AesopResource, SecurityContext,
};
use flowr_policy::{ExecutionRequest, PolicyDecision, PolicyHook, Principal};

/// The A3T baseline policy pack, embedded so the EE binary is self-contained.
pub const A3T_SCHEMA: &str = include_str!("../../../../a3t/policies/a3t.cedarschema");
/// The same base schema in Cedar JSON — the merge base for registry schema
/// fragments (JSON deep-merges cleanly; cedarschema text can't extend an entity).
pub const A3T_SCHEMA_JSON: &str = include_str!("../../../../a3t/policies/a3t.schema.json");
pub const A3T_BASELINE: &str = include_str!("../../../../a3t/policies/baseline.cedar");

/// Recursively merge `over` into `base`: objects merge key-by-key (so entity
/// `shape.attributes` and action maps union), everything else is overwritten by
/// `over`. This gives "fragment extends/overrides base, per entity, all the way
/// down" — exactly the schema-composition semantics we want.
fn deep_merge(base: &mut serde_json::Value, over: serde_json::Value) {
    use serde_json::Value;
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(bv) => deep_merge(bv, v),
                    None => { b.insert(k, v); }
                }
            }
        }
        (b, o) => *b = o,
    }
}

/// Build the effective schema (Cedar JSON string) = embedded base ⊕ optional
/// registry base override ⊕ each per-resource fragment. A fragment that isn't
/// valid JSON, or that yields an unparseable merged schema, is **skipped** (the
/// base and the other fragments survive) — one bad resource can't black out authz.
fn build_merged_schema(base_override: Option<&str>, fragments: &[String]) -> String {
    use serde_json::Value;
    let mut merged: Value = serde_json::from_str(A3T_SCHEMA_JSON)
        .unwrap_or_else(|_| Value::Object(Default::default()));
    if let Some(b) = base_override {
        match serde_json::from_str::<Value>(b) {
            Ok(bv) => deep_merge(&mut merged, bv),
            Err(e) => tracing::warn!("base-schema override is not valid JSON ({e}); ignoring"),
        }
    }
    for (i, f) in fragments.iter().enumerate() {
        match serde_json::from_str::<Value>(f) {
            Ok(fv) => {
                let mut trial = merged.clone();
                deep_merge(&mut trial, fv);
                let s = trial.to_string();
                if AuthzEngine::schema_parses(&s) {
                    merged = trial;
                } else {
                    tracing::warn!("schema fragment #{i} produces an invalid schema; skipping it");
                }
            }
            Err(e) => tracing::warn!("schema fragment #{i} is not valid JSON ({e}); skipping it"),
        }
    }
    merged.to_string()
}

/// A Cedar policy saved in the registry (`type_ref: cedar`): where it lives and
/// its source. In `system`, `a3t` or `default` it is a **platform** policy and
/// applies everywhere; in any other namespace it is that **project's** policy and
/// applies only to calls touching that project.
#[derive(Debug, Clone)]
pub struct NamedPolicy {
    pub namespace: String,
    pub name: String,
    pub source: String,
}

impl NamedPolicy {
    pub fn new(namespace: impl Into<String>, name: impl Into<String>, source: impl Into<String>) -> Self {
        Self { namespace: namespace.into(), name: name.into(), source: source.into() }
    }
    /// `namespace/name`, the label a decision names it by.
    pub fn label(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

/// Namespaces whose policies are the platform's.
pub const PLATFORM_POLICY_NAMESPACES: &[&str] = &["system", "a3t", "default"];

/// Who each Cedar `policyN` is: its label (`@id`, or `<namespace>/<name>`) and,
/// for a rule that says so, why (`@reason`).
#[derive(Debug, Clone, Default)]
struct Labels(Vec<(String, Option<String>)>);

impl Labels {
    /// The labels of the policies in `src`, in the order Cedar numbers them.
    fn of(src: &str, origin: &str) -> Result<Vec<(String, Option<String>)>, String> {
        use std::str::FromStr;
        let set = cedar_policy::PolicySet::from_str(src).map_err(|e| format!("{origin}: {e}"))?;
        let mut ps: Vec<(usize, String, Option<String>)> = set
            .policies()
            .map(|p| {
                let n = p.id().to_string().trim_start_matches("policy").parse::<usize>().unwrap_or(usize::MAX);
                let id = p.annotation("id").map(str::to_string);
                let reason = p.annotation("reason").map(str::to_string);
                (n, id.unwrap_or_else(|| origin.to_string()), reason)
            })
            .collect();
        ps.sort_by_key(|(n, _, _)| *n);
        Ok(ps.into_iter().map(|(_, l, r)| (l, r)).collect())
    }
    fn label(&self, id: &str) -> String {
        id.strip_prefix("policy")
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|n| self.0.get(n))
            .map(|(l, _)| l.clone())
            .unwrap_or_else(|| id.to_string())
    }
    fn reason(&self, id: &str) -> Option<String> {
        id.strip_prefix("policy").and_then(|n| n.parse::<usize>().ok()).and_then(|n| self.0.get(n)).and_then(|(_, r)| r.clone())
    }
    /// `text` with every `policyN` replaced by its label (highest N first, so
    /// `policy1` does not eat the start of `policy12`).
    fn rename(&self, text: &str) -> String {
        let mut out = text.to_string();
        for n in (0..self.0.len()).rev() {
            out = out.replace(&format!("policy{n}"), &format!("'{}'", self.0[n].0));
        }
        out
    }
}

/// One compiled policy set and who its policies are.
struct Pack {
    engine: AuthzEngine,
    labels: Labels,
}

impl Pack {
    /// Baseline + the given policies, in that order, validated against `schema`.
    fn build(baseline: &str, extra: &[(String, String)], schema: &str) -> Result<Self, String> {
        let mut text = String::from(baseline);
        let mut labels = Labels::of(baseline, "baseline")?;
        for (label, src) in extra {
            labels.extend(Labels::of(src, label)?);
            text.push_str("\n// ── ");
            text.push_str(label);
            text.push_str(" ──\n");
            text.push_str(src);
            text.push('\n');
        }
        let labels = Labels(labels);
        let engine = AuthzEngine::new(&text, Some(schema)).map_err(|e| labels.rename(&e.to_string()))?;
        Ok(Self { engine, labels })
    }
}

/// What a project's calls are checked against.
enum ProjectPack {
    Ready(Pack),
    /// Its policy does not compile: its calls are refused (fail closed, for that
    /// project only) until it is fixed.
    Broken(String),
}

/// A [`PolicyHook`] backed by Cedar. Fails **closed**: any engine fault or an
/// unrepresentable request denies.
///
/// The platform set (baseline + platform policies) decides every call; a project
/// with policies of its own has its own set (baseline + platform + its policies),
/// which decides the calls that touch that project.
pub struct CedarHook {
    global: Pack,
    projects: std::collections::HashMap<String, ProjectPack>,
}

impl CedarHook {
    /// Build from explicit Cedar policy + schema source.
    pub fn new(policies_src: &str, schema_src: &str) -> Result<Self, String> {
        Ok(Self { global: Pack::build(policies_src, &[], schema_src)?, projects: Default::default() })
    }

    /// Build from the embedded A3T baseline pack (schema + policies).
    pub fn baseline() -> Result<Self, String> {
        Self::new(A3T_BASELINE, A3T_SCHEMA)
    }

    /// Baseline + unnamed registry policies, all platform-wide (the dry run's
    /// draft path; see [`Self::with_named_policies`] for the real set).
    pub fn with_registry_policies(
        registry_policies: &[String],
        base_override: Option<&str>,
        schema_fragments: &[String],
    ) -> Result<Self, String> {
        let named: Vec<NamedPolicy> = registry_policies
            .iter()
            .enumerate()
            .map(|(i, s)| NamedPolicy::new("default", format!("registry-policy-{i}"), s.clone()))
            .collect();
        Self::with_named_policies(&named, base_override, schema_fragments)
    }

    /// The enforced set: the baked baseline, the platform policies (namespaces
    /// `system`, `a3t`, `default`) for every call, and each project's own
    /// policies for the calls touching it. The baseline guardrails always apply.
    ///
    /// A platform policy that does not compile fails the whole load (fail closed;
    /// the caller keeps the hook it had). A project policy that does not compile
    /// closes that project only: its calls are refused, naming the policy.
    /// Schema: the embedded base ⊕ `base_override` ⊕ `schema_fragments`, falling
    /// back to the embedded base if the merged one rejects the policies.
    pub fn with_named_policies(
        policies: &[NamedPolicy],
        base_override: Option<&str>,
        schema_fragments: &[String],
    ) -> Result<Self, String> {
        let merged = build_merged_schema(base_override, schema_fragments);
        let platform: Vec<(String, String)> = policies
            .iter()
            .filter(|p| PLATFORM_POLICY_NAMESPACES.contains(&p.namespace.as_str()))
            .map(|p| (p.label(), p.source.clone()))
            .collect();
        let (global, schema) = match Pack::build(A3T_BASELINE, &platform, &merged) {
            Ok(g) => (g, merged),
            Err(e) => {
                tracing::warn!("merged registry schema rejected ({e}); using embedded base schema");
                (Pack::build(A3T_BASELINE, &platform, A3T_SCHEMA)?, A3T_SCHEMA.to_string())
            }
        };
        let mut by_ns: std::collections::BTreeMap<&str, Vec<(String, String)>> = Default::default();
        for p in policies.iter().filter(|p| !PLATFORM_POLICY_NAMESPACES.contains(&p.namespace.as_str())) {
            by_ns.entry(p.namespace.as_str()).or_default().push((p.label(), p.source.clone()));
        }
        let mut projects = std::collections::HashMap::new();
        for (ns, own) in by_ns {
            let mut all = platform.clone();
            all.extend(own);
            let pack = match Pack::build(A3T_BASELINE, &all, &schema) {
                Ok(p) => ProjectPack::Ready(p),
                Err(e) => {
                    tracing::error!(project = %ns, error = %e, "a project policy does not compile; that project's calls are refused until it is fixed");
                    ProjectPack::Broken(e)
                }
            };
            projects.insert(ns.to_string(), pack);
        }
        Ok(Self { global, projects })
    }

    /// Check that `policies` load: the platform set, and every project's set.
    /// The error names the policy and says what is wrong. Used before a policy is
    /// saved, so what is stored always compiles.
    pub fn validate(policies: &[NamedPolicy], base_override: Option<&str>, schema_fragments: &[String]) -> Result<(), String> {
        let hook = Self::with_named_policies(policies, base_override, schema_fragments)?;
        for (ns, p) in &hook.projects {
            if let ProjectPack::Broken(e) = p {
                return Err(format!("project {ns}: {e}"));
            }
        }
        Ok(())
    }

    /// The projects with policies of their own.
    pub fn project_count(&self) -> usize {
        self.projects.len()
    }

    /// The set deciding `req`: its project's, else the platform's.
    fn pack_for(&self, req: &ExecutionRequest) -> Result<&Pack, String> {
        match flowr_policy::namespace::request_namespace(req).and_then(|ns| self.projects.get(&ns)) {
            Some(ProjectPack::Ready(p)) => Ok(p),
            Some(ProjectPack::Broken(e)) => Err(e.clone()),
            None => Ok(&self.global),
        }
    }

    /// [`PolicyHook::check`], plus the policies that decided: for an allow, the
    /// permits that matched; for a deny, the forbids (none: nothing permits it).
    /// Labels, not Cedar's positional ids. The Policies page's dry run uses this.
    pub async fn check_explained(&self, req: &ExecutionRequest) -> (PolicyDecision, Vec<String>) {
        let decision = self.check(req).await;
        let ids = match &decision {
            PolicyDecision::Deny { policy_ids, .. } | PolicyDecision::Ask { policy_ids, .. } => policy_ids.clone(),
            PolicyDecision::Allow => {
                let scoped = flowr_policy::namespace::scoped_principal(req).await;
                let ctx = SecurityContext { principal: to_aesop_principal(&scoped), attributes: req.attributes.clone() };
                match (self.pack_for(req), AesopAction::new(req.action.as_str())) {
                    (Ok(pack), Ok(action)) => {
                        let resource = AesopResource {
                            type_name: req.resource.type_name.clone(),
                            id: req.resource.id.clone(),
                            attributes: req.resource.attributes.clone(),
                        };
                        pack.engine.enforce(&ctx, &action, &resource).map(|a| a.policy_ids.iter().map(|i| pack.labels.label(i)).collect()).unwrap_or_default()
                    }
                    _ => Vec::new(),
                }
            }
        };
        (decision, ids)
    }

    /// Load a policy pack from a directory: all `*.cedar` files concatenated as
    /// the policy set, and a `*.cedarschema` if present (else the embedded A3T
    /// schema). This is `security.policies_dir` — policies are deployable and
    /// editable without recompiling the binary.
    pub fn from_dir(dir: &str) -> Result<Self, String> {
        let mut policies = String::new();
        let mut schema: Option<String> = None;
        let entries = std::fs::read_dir(dir).map_err(|e| format!("read {dir}: {e}"))?;
        let mut paths: Vec<_> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        paths.sort();
        for path in paths {
            match path.extension().and_then(|s| s.to_str()) {
                Some("cedar") => {
                    let s = std::fs::read_to_string(&path)
                        .map_err(|e| format!("read {}: {e}", path.display()))?;
                    policies.push_str(&s);
                    policies.push('\n');
                }
                Some("cedarschema") => {
                    schema = Some(
                        std::fs::read_to_string(&path)
                            .map_err(|e| format!("read {}: {e}", path.display()))?,
                    );
                }
                _ => {}
            }
        }
        if policies.trim().is_empty() {
            return Err(format!("no *.cedar policies found in {dir}"));
        }
        Self::new(&policies, schema.as_deref().unwrap_or(A3T_SCHEMA))
    }
}

/// CE `Principal` → aesop `Principal` (identical shape, distinct crates: CE is
/// dependency-light and must not import aesop).
fn to_aesop_principal(p: &Principal) -> AesopPrincipal {
    match p {
        Principal::Human { user_id, roles, .. } => AesopPrincipal::Human {
            user_id: user_id.clone(),
            roles: roles.clone(),
        },
        Principal::Agent { agent_id, on_behalf_of, flow_id, roles, owner, .. } => AesopPrincipal::Agent {
            agent_id: AesopAgentId(agent_id.0.clone()),
            on_behalf_of: on_behalf_of.clone(),
            roles: roles.clone(),
            owner: owner.clone(),
            flow_id: flow_id.clone(),
        },
        Principal::Service { service_id, .. } => AesopPrincipal::Service {
            service_id: service_id.clone(),
        },
    }
}

impl CedarHook {
    /// The human who could answer an approval prompt for this principal, as a
    /// Cedar principal carrying their *own* authority.
    ///
    /// Only an agent acting for a human has one. A human needs no approver (they
    /// are already the decision), and a service principal has nobody behind it —
    /// an event-triggered run must stand on its own policy rather than borrow a
    /// human's, which is why `Service` returns `None` and its denials stay
    /// denials.
    ///
    /// The approver's roles are the agent's inherited `roles`. That is sound
    /// because inheritance is attenuate-only (`Principal::agent_for`): the agent
    /// never holds a role its invoker lacked, so reconstructing the invoker from
    /// them cannot over-state what the human can do.
    fn approver_of(&self, p: &Principal) -> Option<AesopPrincipal> {
        match p {
            Principal::Agent { on_behalf_of, roles, .. } => {
                let user_id = on_behalf_of.clone()?;
                Some(AesopPrincipal::Human { user_id, roles: roles.clone() })
            }
            _ => None,
        }
    }
}

/// A refusal in words: what was refused, which policy refused it and what access
/// it needs (the rule's `@reason`), or that nothing permits it for these roles.
fn deny_reason(p: &Principal, action: &str, rtype: &str, rid: &str, ids: &[String], why: &[String]) -> String {
    let what = format!("{action} on {rtype} '{rid}'");
    let roles = match p {
        Principal::Human { roles, .. } | Principal::Agent { roles, .. } if !roles.is_empty() => roles.join(", "),
        Principal::Agent { .. } => "an agent's".into(),
        Principal::Service { .. } => "the service's".into(),
        _ => "no".into(),
    };
    if ids.is_empty() {
        return format!("Refused: no policy permits {what} with {roles} role(s) in this project.");
    }
    let by = ids.join(", ");
    if why.is_empty() {
        format!("Refused by policy {by}: {what}.")
    } else {
        format!("Refused by policy {by}: {what}. {}", why.join(" "))
    }
}

/// The template keys that describe the agent's **capability ceiling** — what it
/// was configured to be able to do.
///
/// Everything else under `template` stays: `tags`, `policyRef` and
/// `runtimePolicy` are how InfoSec-authored policies ATTACH to a flow (§6.3).
/// Removing those would let an approver escape, say, a "no external HTTP from
/// PII-tagged flows" rule — org policy, not agent configuration.
const CEILING_KEYS: &[&str] = &["exposedTools", "enabledTools"];

/// Re-cast an attribute bag for evaluation as the human behind an agent.
///
/// Drops only the capability-ceiling keys, and only when that human OWNS the
/// agent. The ceiling is configuration its owner is the authority over, so an
/// owner can be asked to widen it; anyone else runs inside it exactly as the
/// agent does, and a ceiling violation stays a denial for them.
fn as_approver_attributes(
    attrs: &serde_json::Map<String, serde_json::Value>,
    is_owner: bool,
) -> serde_json::Map<String, serde_json::Value> {
    let mut out = attrs.clone();
    if !is_owner {
        return out;
    }
    if let Some(serde_json::Value::Object(template)) = out.get_mut("template") {
        for k in CEILING_KEYS {
            template.remove(*k);
        }
    }
    out
}

#[async_trait::async_trait]
impl PolicyHook for CedarHook {
    async fn check(&self, req: &ExecutionRequest) -> PolicyDecision {
        // The roles a user holds in the project this call touches, not their
        // global ones (.ai/A3T_ORG_SSO_SPEC.md R6/R7): admin in their own org,
        // viewer in another, whatever their Keycloak realm role says.
        let scoped = flowr_policy::namespace::scoped_principal(req).await;
        let ctx = SecurityContext {
            principal: to_aesop_principal(&scoped),
            // E2 #2a: the flow/agent template data (`principal.template.*`) rides
            // here so InfoSec-owned policy resources can gate on it.
            attributes: req.attributes.clone(),
        };

        // The action shape was already validated at the CE edge; re-validate to
        // convert into the aesop type. A malformed action fails closed.
        let action = match AesopAction::new(req.action.as_str()) {
            Ok(a) => a,
            Err(e) => {
                return PolicyDecision::Deny {
                    reason: format!("invalid action '{}': {e}", req.action.as_str()),
                    policy_ids: vec![],
                }
            }
        };

        let resource = AesopResource {
            type_name: req.resource.type_name.clone(),
            id: req.resource.id.clone(),
            attributes: req.resource.attributes.clone(),
        };

        let pack = match self.pack_for(req) {
            Ok(p) => p,
            Err(e) => {
                return PolicyDecision::Deny {
                    reason: format!("This project's policy doesn't compile, so its calls are refused until a SecurityAdmin fixes it: {e}"),
                    policy_ids: vec![],
                }
            }
        };
        match pack.engine.enforce(&ctx, &action, &resource) {
            Ok(_allowed) => PolicyDecision::Allow,
            Err(AesopError::Denied { policy_ids: raw_ids, action: denied_action, resource_type, resource_id }) => {
                let policy_ids: Vec<String> = raw_ids.iter().map(|i| pack.labels.label(i)).collect();
                let why: Vec<String> = raw_ids.iter().filter_map(|i| pack.labels.reason(i)).collect();
                // ── Deny, or a question? ──
                //
                // Cedar only ever answers allow/deny; `Ask` is derived, not
                // declared. Re-run the SAME request as the human the agent acts
                // for. If the human would have been allowed, the agent is merely
                // exceeding a delegation the human is entitled to widen — that is
                // a question, and chat can put it to them. If the human would
                // also be denied, it stays a denial.
                //
                // This is what keeps approval from becoming privilege escalation:
                // nobody is ever asked to grant authority they do not already
                // hold, so the worst a prompt can do is re-delegate what the
                // approver could have done by hand.
                if let Some(approver) = self.approver_of(&req.principal) {
                    // Only the agent's OWNER may be asked to widen its ceiling.
                    // For anyone else the ceiling binds exactly as it binds the
                    // agent, so the denial stands — otherwise any user who could
                    // run an agent could talk it out of its configured limits.
                    let is_owner = match (&req.principal, req.principal.on_behalf_of()) {
                        (Principal::Agent { owner: Some(owner), .. }, Some(human)) => owner == human,
                        _ => false,
                    };
                    let approver_ctx = SecurityContext {
                        principal: approver,
                        // The agent's CAPABILITY CEILING is dropped for its owner,
                        // and that is the crux of the mechanism.
                        //
                        // `exposedTools`/`enabledTools` describe how the agent was
                        // configured, not what the human may do. Passing them through
                        // made the ceiling forbid fire against the approver too, so
                        // removing a tool from an agent produced a flat denial and
                        // never a prompt — the one case this feature exists to serve.
                        //
                        // Only the ceiling keys go, and only for the owner. `tags`,
                        // `policyRef` and `runtimePolicy` are how InfoSec policies
                        // attach to a flow, so they stay — otherwise an approver could
                        // wave away a rule like "no external HTTP from PII flows".
                        attributes: as_approver_attributes(&req.attributes, is_owner),
                    };
                    if pack.engine.enforce(&approver_ctx, &action, &resource).is_ok() {
                        return PolicyDecision::Ask {
                            reason: format!(
                                "{denied_action} on {resource_type}::{resource_id} is outside this agent's \
                                 grants, but you can authorize it"
                            ),
                            policy_ids,
                            scopes: vec![
                                flowr_policy::GrantScope::Once,
                                flowr_policy::GrantScope::Session,
                                flowr_policy::GrantScope::Agent,
                            ],
                        };
                    }
                }
                PolicyDecision::Deny {
                    reason: deny_reason(&scoped, &denied_action, &resource_type, &resource_id, &policy_ids, &why),
                    policy_ids,
                }
            }
            // Engine faults deny (fail closed) — an allow is never fabricated
            // from a fault path.
            Err(e) => PolicyDecision::Deny {
                reason: format!("policy engine fault: {e}"),
                policy_ids: vec![],
            },
        }
    }
}
