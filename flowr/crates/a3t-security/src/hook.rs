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

/// A policy whose name starts with this runs in shadow mode (GOV-2): it is
/// evaluated after the deciding set, and what it would refuse is logged as
/// `policy:shadow`, never enforced.
pub const SHADOW_PREFIX: &str = "shadow:";

/// One compiled policy set and who its policies are.
struct Pack {
    engine: AuthzEngine,
    labels: Labels,
    /// The `needs_approval` rules (`@approval` forbids), kept out of the
    /// deciding set: after an allow, a match asks a person (GOV-8).
    approvals: Option<Approvals>,
    /// Shadow-mode files (GOV-2) and rules that only log: a permit-all plus
    /// their forbids.
    shadow: Option<Approvals>,
    /// Rules with a counted condition (`@count`: rate, lockout, first use,
    /// tokens): a permit-all plus their forbids without the count; a match
    /// asks the condition source whether the count is met.
    counted: Option<Approvals>,
    /// Positional ids (`policyN`) of `@firm` statements (org rules): their
    /// refusal of an agent stands; it never becomes a delegation prompt.
    firm: std::collections::HashSet<String>,
    /// sha256 of the policy and schema source, first 16 hex (AUD-2).
    version: String,
}

/// The approval set: a permit-all plus the `@approval` forbids, so it denies
/// exactly when an approval rule matches.
struct Approvals {
    engine: AuthzEngine,
    labels: Labels,
    /// Each policy's terms, by Cedar position.
    meta: Vec<Meta>,
}

/// A statement's annotations the hook acts on.
#[derive(Debug, Clone, Default)]
struct Meta {
    /// `@approval`: asks a person; the timeout in seconds.
    approval: Option<Option<u64>>,
    /// `@approver`: who answers.
    approver: Option<String>,
    /// `@count`: the counted conditions (all must be met).
    count: Vec<serde_json::Value>,
}

impl Meta {
    fn of(p: &cedar_policy::Policy) -> Self {
        Meta {
            approval: p.annotation("approval").map(|v| v.parse::<u64>().ok()),
            approver: p.annotation("approver").map(str::to_string),
            count: p.annotation("count").and_then(|v| serde_json::from_str::<Vec<serde_json::Value>>(v).ok()).unwrap_or_default(),
        }
    }
}

/// Each statement's terms, by Cedar position (`policy0`, `policy1`, …).
fn metas(full: &str) -> Result<Vec<Meta>, String> {
    use std::str::FromStr;
    let set = cedar_policy::PolicySet::from_str(full).map_err(|e| e.to_string())?;
    let mut t: Vec<(usize, Meta)> = set.policies()
        .map(|p| (p.id().to_string().trim_start_matches("policy").parse::<usize>().unwrap_or(usize::MAX), Meta::of(p)))
        .collect();
    t.sort_by_key(|(n, _)| *n);
    Ok(t.into_iter().map(|(_, m)| m).collect())
}

/// A permit-all plus `src`'s forbids: it denies exactly when one of them matches.
fn side_set(base_id: &str, src: &str, labels: Vec<(String, Option<String>)>, schema: &str) -> Result<Option<Approvals>, String> {
    if src.trim().is_empty() {
        return Ok(None);
    }
    let base = format!("@id(\"{base_id}\")\npermit(principal, action, resource);\n\n");
    let full = format!("{base}{src}");
    let mut l = Labels::of(&base, base_id)?;
    l.extend(labels);
    let labels = Labels(l);
    let engine = AuthzEngine::new(&full, Some(schema)).map_err(|e| labels.rename(&e.to_string()))?;
    Ok(Some(Approvals { engine, labels, meta: metas(&full)? }))
}

/// A policy source sorted by what the hook does with each statement.
#[derive(Default)]
struct Split {
    /// Decide (refuse).
    main: String,
    /// `@approval` (no count): ask a person.
    approval: String,
    /// `@count`: checked against A3T's counts.
    counted: String,
    /// `@log`: logged as what would be refused, never enforced.
    log: String,
}

fn split(src: &str, origin: &str) -> Result<Split, String> {
    if !["@approval", "@count", "@log"].iter().any(|a| src.contains(a)) {
        return Ok(Split { main: src.to_string(), ..Default::default() });
    }
    use std::str::FromStr;
    let set = cedar_policy::PolicySet::from_str(src).map_err(|e| format!("{origin}: {e}"))?;
    let mut ps: Vec<(usize, &cedar_policy::Policy)> = set
        .policies()
        .map(|p| (p.id().to_string().trim_start_matches("policy").parse::<usize>().unwrap_or(usize::MAX), p))
        .collect();
    ps.sort_by_key(|(n, _)| *n);
    let mut out = Split::default();
    for (_, p) in ps {
        let special = ["approval", "count", "log"].iter().any(|a| p.annotation(a).is_some());
        if special && p.effect() != cedar_policy::Effect::Forbid {
            return Err(format!("{origin}: an @approval, @count or @log statement must be a forbid"));
        }
        let to = if p.annotation("log").is_some() {
            &mut out.log
        } else if p.annotation("count").is_some() {
            &mut out.counted
        } else if p.annotation("approval").is_some() {
            &mut out.approval
        } else {
            &mut out.main
        };
        to.push_str(&p.to_string());
        to.push_str("\n\n");
    }
    Ok(out)
}

impl Pack {
    /// Baseline + the given policies, in that order, validated against `schema`.
    fn build(baseline: &str, extra: &[(String, String)], schema: &str) -> Result<Self, String> {
        let mut text = String::from(baseline);
        let mut labels = Labels::of(baseline, "baseline")?;
        let (mut appr_text, mut appr_labels) = (String::new(), Vec::new());
        let (mut shadow_text, mut shadow_labels) = (String::new(), Vec::new());
        let (mut count_text, mut count_labels) = (String::new(), Vec::new());
        for (label, src) in extra {
            if label.split_once('/').is_some_and(|(_, n)| n.starts_with(SHADOW_PREFIX)) {
                shadow_labels.extend(Labels::of(src, label)?);
                shadow_text.push_str(src);
                shadow_text.push('\n');
                continue;
            }
            let sp = split(src, label)?;
            if !sp.main.trim().is_empty() {
                labels.extend(Labels::of(&sp.main, label)?);
                text.push_str("\n// ── ");
                text.push_str(label);
                text.push_str(" ──\n");
                text.push_str(&sp.main);
                text.push('\n');
            }
            for (part, t, l) in [(&sp.approval, &mut appr_text, &mut appr_labels), (&sp.log, &mut shadow_text, &mut shadow_labels), (&sp.counted, &mut count_text, &mut count_labels)] {
                if !part.trim().is_empty() {
                    l.extend(Labels::of(part, label)?);
                    t.push_str(part);
                }
            }
        }
        let labels = Labels(labels);
        let engine = AuthzEngine::new(&text, Some(schema)).map_err(|e| labels.rename(&e.to_string()))?;
        let firm = {
            use std::str::FromStr;
            cedar_policy::PolicySet::from_str(&text).map_err(|e| e.to_string())?
                .policies().filter(|p| p.annotation("firm").is_some()).map(|p| p.id().to_string()).collect()
        };
        let approvals = side_set("approval-base", &appr_text, appr_labels, schema)?;
        let shadow = side_set("shadow-base", &shadow_text, shadow_labels, schema)?;
        let counted = side_set("count-base", &count_text, count_labels, schema)?;
        // The version covers every statement, wherever it is evaluated.
        for (name, t) in [("approvals", &appr_text), ("counted", &count_text)] {
            if !t.trim().is_empty() {
                text.push_str(&format!("\n// ── {name} ──\n"));
                text.push_str(t);
            }
        }
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(text.as_bytes());
        h.update(b"\0");
        h.update(schema.as_bytes());
        let version = h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect();
        Ok(Self { engine, labels, approvals, shadow, counted, firm, version })
    }

    /// GOV-2: what the shadow files would refuse that the enforced set allowed —
    /// their ids and reasons. Never changes the decision.
    fn shadow_refusal(&self, ctx: &SecurityContext, action: &AesopAction, resource: &AesopResource) -> Option<(Vec<String>, Vec<String>)> {
        let a = self.shadow.as_ref()?;
        match a.engine.enforce(ctx, action, resource) {
            Err(AesopError::Denied { policy_ids, .. }) if !policy_ids.is_empty() => Some((
                policy_ids.iter().map(|i| a.labels.label(i).replace(SHADOW_PREFIX, "")).collect(),
                policy_ids.iter().filter_map(|i| a.labels.reason(i)).collect(),
            )),
            _ => None,
        }
    }

    /// After an allow: does a `needs_approval` rule cover this call? Its ids,
    /// reasons, the shortest timeout among them and who answers.
    fn approval_needed(&self, ctx: &SecurityContext, action: &AesopAction, resource: &AesopResource) -> Option<(Vec<String>, Vec<String>, flowr_policy::RequiredApproval)> {
        let a = self.approvals.as_ref()?;
        match a.engine.enforce(ctx, action, resource) {
            Ok(_) => None,
            Err(AesopError::Denied { policy_ids, .. }) if !policy_ids.is_empty() => {
                let ids = policy_ids.iter().map(|i| a.labels.label(i)).collect();
                let why = policy_ids.iter().filter_map(|i| a.labels.reason(i)).collect();
                let metas: Vec<&Meta> = policy_ids.iter().filter_map(|i| a.meta_of(i)).collect();
                Some((ids, why, terms(&metas)))
            }
            // A fault in the approval set asks rather than letting it through.
            Err(_) => Some((vec!["approval-set".into()], Vec::new(), Default::default())),
        }
    }

    /// After an allow: the counted rules this call matches (their count aside),
    /// as `(id, reason, terms)`. A fault refuses (`Err`).
    fn counted_matches(&self, ctx: &SecurityContext, action: &AesopAction, resource: &AesopResource) -> Result<Vec<(String, Option<String>, Meta)>, String> {
        let Some(a) = self.counted.as_ref() else { return Ok(Vec::new()) };
        match a.engine.enforce(ctx, action, resource) {
            Ok(_) => Ok(Vec::new()),
            Err(AesopError::Denied { policy_ids, .. }) => Ok(policy_ids.iter()
                .filter_map(|i| a.meta_of(i).map(|m| (a.labels.label(i), a.labels.reason(i), m.clone())))
                .collect()),
            Err(e) => Err(format!("the counted rules could not be checked: {e}")),
        }
    }
}

impl Approvals {
    fn meta_of(&self, id: &str) -> Option<&Meta> {
        id.strip_prefix("policy").and_then(|n| n.parse::<usize>().ok()).and_then(|n| self.meta.get(n))
    }
}

/// The approval terms of the rules that asked: the shortest timeout, and the
/// first named approver.
fn terms(metas: &[&Meta]) -> flowr_policy::RequiredApproval {
    flowr_policy::RequiredApproval {
        timeout_secs: metas.iter().filter_map(|m| m.approval.flatten()).min(),
        approver: metas.iter().find_map(|m| m.approver.clone()),
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
    /// Don't log shadow refusals (a replay re-evaluates past decisions).
    quiet: bool,
}

impl CedarHook {
    /// Build from explicit Cedar policy + schema source.
    pub fn new(policies_src: &str, schema_src: &str) -> Result<Self, String> {
        Ok(Self { global: Pack::build(policies_src, &[], schema_src)?, projects: Default::default(), quiet: false })
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
        Ok(Self { global, projects, quiet: false })
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

    /// The same hook, logging nothing of its own (for a replay, GOV-2).
    pub fn quiet(mut self) -> Self {
        self.quiet = true;
        self
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
                let (attributes, context) = facts(req, &scoped, &req.attributes).await;
                let ctx = SecurityContext { principal: to_aesop_principal(&scoped), attributes, context };
                match (self.pack_for(req), AesopAction::new(req.action.as_str())) {
                    (Ok(pack), Ok(action)) => {
                        let resource = AesopResource {
                            type_name: req.resource.type_name.clone(),
                            id: req.resource.id.clone(),
                            attributes: req.resource.attributes.clone(),
                            tags: numeric_args(&req.params),
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

/// Log what a shadow file would have refused (GOV-2), as `policy:shadow` on
/// the same resource; the call itself goes ahead.
fn log_shadow(req: &ExecutionRequest, ids: &[String], why: &[String]) {
    let project = flowr_policy::namespace::request_namespace(req);
    let reason = format!("shadow: would refuse {} on {}::{} ({}). {}", req.action.as_str(), req.resource.type_name, req.resource.id, ids.join(", "), why.join(" "));
    flowr_policy::audit::event_decision(
        &req.principal, "policy:shadow", &req.resource.type_name, &req.resource.id, project.as_deref(), None,
        &PolicyDecision::Deny { reason: reason.trim_end().to_string(), policy_ids: ids.to_vec() },
        Some(&format!("not enforced; the call was allowed ({})", req.action.as_str())),
    );
}

/// What a `needs_approval` rule makes of an allowed call: a question for the
/// person behind the agent (or its owner), asked every time; with nobody to
/// ask, a refusal.
fn required_ask(p: &Principal, action: &AesopAction, resource: &AesopResource, policy_ids: Vec<String>, why: Vec<String>, terms: flowr_policy::RequiredApproval) -> PolicyDecision {
    let what = format!("{} on {}::{}", action.as_str(), resource.type_name, resource.id);
    let because = if why.is_empty() { String::new() } else { format!(" {}", why.join(" ")) };
    // A named approver (a role or group) can answer for any agent; otherwise
    // it is the person behind the agent (or its owner).
    let someone = matches!(p, Principal::Agent { .. }) && (terms.approver.is_some() || p.on_behalf_of().is_some() || p.owner().is_some());
    if !someone {
        return PolicyDecision::Deny {
            reason: format!("{what} needs a person's approval (policy {}), and nobody stands behind this call to give it.{because}", policy_ids.join(", ")),
            policy_ids,
        };
    }
    PolicyDecision::Ask {
        reason: format!("{what} needs a person's approval (policy {}).{because}", policy_ids.join(", ")),
        policy_ids,
        scopes: vec![flowr_policy::GrantScope::Once],
        required: Some(terms),
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

/// The call's number arguments, as the Tool's tags (`getTag`, whole numbers).
/// MCP calls carry them under `arguments`. A fraction rounds up, so "over 1000"
/// reads 1000.50 as over and 999.50 as not.
fn numeric_args(params: &serde_json::Value) -> std::collections::BTreeMap<String, i64> {
    let mut out = std::collections::BTreeMap::new();
    let mut take = |m: &serde_json::Map<String, serde_json::Value>| {
        for (k, v) in m {
            let n = match v {
                serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f.ceil().clamp(i64::MIN as f64, i64::MAX as f64) as i64)),
                serde_json::Value::String(s) => s.trim().parse::<i64>().ok(),
                _ => None,
            };
            if let Some(n) = n {
                out.insert(k.clone(), n);
            }
        }
    };
    if let Some(m) = params.as_object() {
        take(m);
        if let Some(a) = m.get("arguments").and_then(|v| v.as_object()) {
            take(a);
        }
    }
    out
}

/// The one tenant role held where the call lands: the highest of Admin,
/// Operator, Viewer (roles stack, so a rule naming one level matches on this).
pub fn level_of(roles: &[String]) -> Option<&'static str> {
    ["Admin", "Operator", "Viewer"].into_iter().find(|l| roles.iter().any(|r| r == l))
}

/// What a rule may read about this call besides who and what: the principal's
/// facts (merged into `principal.*`) and the request context (`context.*`).
async fn facts(req: &ExecutionRequest, scoped: &Principal, attributes: &serde_json::Map<String, serde_json::Value>)
    -> (serde_json::Map<String, serde_json::Value>, serde_json::Map<String, serde_json::Value>) {
    use serde_json::{json, Value};
    let mut attrs = attributes.clone();
    // Facts a choke point supplies for this call (e.g. `untrusted_seen` from an
    // agent turn) arrive under `context`; they are context, not principal data.
    let supplied = match attrs.remove("context") {
        Some(Value::Object(m)) => m,
        _ => Default::default(),
    };
    if let Some(l) = level_of(scoped.roles()) {
        attrs.insert("level".into(), json!(l));
    }
    if let Some(by) = scoped.impersonated_by() {
        attrs.insert("impersonated_by".into(), json!(by));
    }
    let session = scoped.session();
    if let Some(sf) = session {
        if !sf.groups.is_empty() {
            attrs.insert("groups".into(), json!(sf.groups));
        }
        if let Some(e) = &sf.email {
            attrs.insert("email".into(), json!(e.to_lowercase()));
        }
    }
    let mut ctx = serde_json::Map::new();
    for k in ["untrusted_seen", "args_pii"] {
        if let Some(b) = supplied.get(k).and_then(|v| v.as_bool()) {
            ctx.insert(k.into(), json!(b));
        }
    }
    let src = flowr_policy::conditions::condition_source();
    let ns = flowr_policy::namespace::request_namespace(req);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let offset = src.utc_offset_minutes(ns.as_deref()).await as i64;
    let (day, weekday, minute) = flowr_policy::conditions::civil(now + offset * 60);
    ctx.insert("local".into(), json!({ "day": day, "weekday": weekday, "minute": minute }));
    if let Some(s) = src.since_deploy_s(ns.as_deref()).await {
        ctx.insert("since_deploy_s".into(), json!(s));
    }
    if !ctx.contains_key("args_pii") && !req.params.is_null() {
        ctx.insert("args_pii".into(), json!(src.args_pii(&req.params)));
    }
    if let Some(sf) = session {
        if let Some(ip) = sf.ip.as_deref().filter(|ip| ip.parse::<std::net::IpAddr>().is_ok()) {
            ctx.insert("ip".into(), json!({ "__extn": { "fn": "ip", "arg": ip } }));
        }
        if let Some(c) = &sf.channel {
            ctx.insert("channel".into(), json!(c));
        }
        let mut auth = json!({ "second_factor": sf.second_factor, "org_idp": sf.org_idp });
        if let Some(t) = sf.auth_time {
            auth["age_s"] = json!((now - t).max(0));
        }
        ctx.insert("auth".into(), auth);
    }
    (attrs, ctx)
}

impl CedarHook {
    /// The counted rules this (allowed) call matches: each one whose counted
    /// conditions are all met refuses it, or asks. Otherwise the call is
    /// counted. A count that can't be read refuses (fail closed). A replay
    /// (`quiet`) neither reads nor writes counts.
    async fn counted(&self, pack: &Pack, req: &ExecutionRequest, scoped: &Principal, ctx: &SecurityContext, action: &AesopAction, resource: &AesopResource) -> Option<PolicyDecision> {
        if self.quiet {
            return None;
        }
        let matches = match pack.counted_matches(ctx, action, resource) {
            Ok(m) if m.is_empty() => return None,
            Ok(m) => m,
            Err(e) => return Some(PolicyDecision::Deny { reason: format!("Refused: {e}"), policy_ids: vec!["count-set".into()] }),
        };
        let src = flowr_policy::conditions::condition_source();
        let key = |rule: &str| flowr_policy::conditions::CountKey {
            rule: rule.to_string(),
            person: flowr_policy::namespace::acting_user(scoped).map(str::to_string),
            agent: match scoped { Principal::Agent { agent_id, .. } => Some(agent_id.0.clone()), _ => None },
            namespace: flowr_policy::namespace::request_namespace(req),
            resource: format!("{}::{}", resource.type_name, resource.id),
            action: action.as_str().to_string(),
        };
        let mut met: Vec<(String, Option<String>, Meta)> = Vec::new();
        let mut not_met: Vec<(String, Meta)> = Vec::new();
        for (id, why, meta) in matches {
            let k = key(&rule_of(&id));
            let mut all = !meta.count.is_empty();
            for spec in &meta.count {
                match src.over(spec, &k).await {
                    Ok(true) => {}
                    Ok(false) => { all = false; break; }
                    Err(e) => return Some(PolicyDecision::Deny {
                        reason: format!("Refused: rule {id} needs a count A3T could not read ({e})."),
                        policy_ids: vec![id],
                    }),
                }
            }
            if all { met.push((id, why, meta)); } else { not_met.push((id, meta)); }
        }
        // This call goes on (or waits) under the rules whose count was not met:
        // it counts towards them.
        if met.iter().all(|(_, _, m)| m.approval.is_some()) {
            for (id, meta) in &not_met {
                let k = key(&rule_of(id));
                for spec in &meta.count {
                    src.record(spec, &k).await;
                }
            }
        }
        if met.is_empty() {
            return None;
        }
        let ids: Vec<String> = met.iter().map(|(i, _, _)| i.clone()).collect();
        let why: Vec<String> = met.iter().filter_map(|(_, w, _)| w.clone()).collect();
        if met.iter().all(|(_, _, m)| m.approval.is_some()) {
            let metas: Vec<&Meta> = met.iter().map(|(_, _, m)| m).collect();
            // A first use that waits is counted: the next one doesn't ask again.
            for (id, _, meta) in &met {
                for spec in meta.count.iter().filter(|s| s.get("kind").and_then(|k| k.as_str()) == Some("first_use")) {
                    src.record(spec, &key(&rule_of(id))).await;
                }
            }
            return Some(required_ask(&req.principal, action, resource, ids, why, terms(&metas)));
        }
        Some(PolicyDecision::Deny {
            reason: deny_reason(scoped, action.as_str(), &resource.type_name, &resource.id, &ids, &why),
            policy_ids: ids,
        })
    }
}

/// `<org>/<policy>/r3.1.2p` → `<org>/<policy>/r3`: counts are kept per rule.
fn rule_of(id: &str) -> String {
    match id.rsplit_once("/r") {
        Some((head, tail)) => format!("{head}/r{}", tail.split('.').next().unwrap_or(tail)),
        None => id.to_string(),
    }
}

#[async_trait::async_trait]
impl PolicyHook for CedarHook {
    fn version_for(&self, req: &ExecutionRequest) -> Option<String> {
        self.pack_for(req).ok().map(|p| p.version.clone())
    }

    async fn check(&self, req: &ExecutionRequest) -> PolicyDecision {
        // The roles a user holds in the project this call touches, not their
        // global ones (.ai/A3T_ORG_SSO_SPEC.md R6/R7): admin in their own org,
        // viewer in another, whatever their Keycloak realm role says.
        let scoped = flowr_policy::namespace::scoped_principal(req).await;
        let (attributes, context) = facts(req, &scoped, &req.attributes).await;
        let ctx = SecurityContext {
            principal: to_aesop_principal(&scoped),
            // E2 #2a: the flow/agent template data (`principal.template.*`) rides
            // here so InfoSec-owned policy resources can gate on it — with the
            // rule facts (level, groups, email) and the request context.
            attributes,
            context: context.clone(),
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
            tags: numeric_args(&req.params),
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
            Ok(_allowed) => {
                if let Some((ids, why)) = pack.shadow_refusal(&ctx, &action, &resource).filter(|_| !self.quiet) {
                    log_shadow(req, &ids, &why);
                }
                // Counted rules (rate, lockout, first use, tokens): those this
                // call matches, if their count is met, refuse or ask.
                if let Some(d) = self.counted(pack, req, &scoped, &ctx, &action, &resource).await {
                    return d;
                }
                match pack.approval_needed(&ctx, &action, &resource) {
                None => PolicyDecision::Allow,
                Some((policy_ids, why, terms)) => required_ask(&req.principal, &action, &resource, policy_ids, why, terms),
            }},
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
                // An org rule's refusal stands: it is the org's decision, not a
                // limit of this agent that the person behind it may widen.
                let firm = raw_ids.iter().any(|i| pack.firm.contains(i));
                if let Some(approver) = self.approver_of(&req.principal).filter(|_| !firm) {
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
                        attributes: as_approver_attributes(&ctx.attributes, is_owner),
                        context: ctx.context.clone(),
                    };
                    if pack.engine.enforce(&approver_ctx, &action, &resource).is_ok() {
                        let reason = format!(
                            "{denied_action} on {resource_type}::{resource_id} is outside this agent's \
                             grants, but you can authorize it"
                        );
                        // An approval rule covers it too: one question, asked
                        // every time, as the rule says.
                        if let Some((ids, why, terms)) = pack.approval_needed(&ctx, &action, &resource) {
                            let mut all = policy_ids;
                            all.extend(ids);
                            return PolicyDecision::Ask {
                                reason: format!("{reason}; a rule also asks a person every time. {}", why.join(" ")).trim_end().to_string(),
                                policy_ids: all,
                                scopes: vec![flowr_policy::GrantScope::Once],
                                required: Some(terms),
                            };
                        }
                        return PolicyDecision::Ask {
                            reason,
                            policy_ids,
                            scopes: vec![
                                flowr_policy::GrantScope::Once,
                                flowr_policy::GrantScope::Session,
                                flowr_policy::GrantScope::Agent,
                            ],
                            required: None,
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
