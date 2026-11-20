//! Config-to-code policy: org policy and role files (YAML) compiled to Cedar.
//!
//! SecOps build rules in the console (or experts write Cedar); A3T saves them as
//! files in the org's policy repo, and this module turns a file into Cedar
//! `forbid` statements. Org policy can only **narrow** what the A3T baseline
//! allows: every plain rule becomes a `forbid`, and an expert `cedar:` block may
//! hold `forbid`s only. Names come from the permission catalogue
//! (`a3t/policies/catalogue.yaml`).
//!
//! ```yaml
//! kind: a3t/policy
//! name: no-external-llm
//! applies_to: { projects: [acme-crm], tags: [pii], flows: [acme-crm/refunds] }
//! rules:
//!   - who: { roles: [Operator], agents: any }
//!     cannot: [models.use]
//!     on: { models: { not_in: [acme/claude-sonnet] } }
//!     reason: Only the org's approved models.
//! cedar: |
//!   forbid(principal, action == Action::"mcp:call", resource) when { ... };
//! ```
//!
//! A role file is a baseline role minus permissions; it compiles to forbids on
//! principals holding that role:
//!
//! ```yaml
//! kind: a3t/role
//! name: support-operator
//! extends: Operator
//! remove: [memory.change, tools.delete_*]
//! ```

use serde::Serialize;
use std::str::FromStr;

use flowr_policy::catalogue::{catalogue, Catalogue, Permission};

/// The baseline roles an org role can extend (and a rule can name).
pub const BASELINE_ROLES: &[&str] = &["Admin", "Operator", "Viewer"];
const TOOL_ACTIONS: &str = r#"[Action::"mcp:call", Action::"tool:invoke"]"#;

pub use flowr_policy::policy_file::{kind_of_path, AccessFile, AgentsSel, AppliesTo, InOrNot, On, PolicyFile, RoleFile, Rule, Who};

/// Where a compiled policy is placed (which policy sets carry it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// Every project of the org.
    Org,
    /// These projects only.
    Projects(Vec<String>),
}

/// An app a rule hides in the console, and for whom (a Cedar-free summary).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HiddenApp {
    pub app: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Compiled {
    /// `<org>/<name>`.
    pub name: String,
    /// The Cedar statements, each with `@id` and `@reason`.
    pub cedar: String,
    pub placement: Placement,
    pub hidden_apps: Vec<HiddenApp>,
}

fn q(s: &str) -> String {
    serde_json::to_string(s).expect("a string serializes")
}

fn set(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|s| q(s)).collect::<Vec<_>>().join(", "))
}

fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 63 && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && n.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
}

/// A glob as a Cedar `like` pattern (`*` stays; a literal `*` can't be asked for).
fn like(glob: &str) -> Result<String, String> {
    if glob.is_empty() || glob.contains('"') || glob.contains('\\') {
        return Err(format!("{glob:?} is not a tool name pattern"));
    }
    Ok(format!("\"{glob}\""))
}

/// What one `cannot` entry forbids: (action clause for the head, resource
/// condition) pairs, or an app the console hides.
enum Target {
    Cedar(Vec<(String, Option<String>)>),
    App(String),
}

fn target_of(cat: &Catalogue, item: &str) -> Result<Target, String> {
    if let Some(glob) = item.strip_prefix("tools.").filter(|g| *g != "external") {
        if cat.permissions.iter().all(|p| p.id != item) {
            let cond = format!("resource is Tool && resource has name && resource.name like {}", like(glob)?);
            return Ok(Target::Cedar(vec![(format!("action in {TOOL_ACTIONS}"), Some(cond))]));
        }
    }
    let p: &Permission = cat.permissions.iter().find(|p| p.id == item)
        .ok_or_else(|| format!("{item:?} is not in the permission catalogue"))?;
    if p.enforced_by.as_deref() == Some("console") {
        return Ok(Target::App(p.id.trim_start_matches("apps.").to_string()));
    }
    let mut parts = Vec::new();
    if p.id == "tools.external" {
        // Tools of connected MCP servers: anything that is not a built-in tool.
        let builtin: Vec<String> = cat.permissions.iter().flat_map(|p| p.tools.iter().cloned()).collect();
        parts.push((format!("action in {TOOL_ACTIONS}"),
            Some(format!("resource is Tool && resource has name && !({}.contains(resource.name))", set(&builtin)))));
        return Ok(Target::Cedar(parts));
    }
    if !p.tools.is_empty() {
        parts.push((format!("action in {TOOL_ACTIONS}"),
            Some(format!("resource is Tool && resource has name && {}.contains(resource.name)", set(&p.tools)))));
    }
    if !p.actions.is_empty() {
        let acts = p.actions.iter().map(|a| format!("Action::{}", q(a))).collect::<Vec<_>>().join(", ");
        parts.push((format!("action in [{acts}]"), None));
    }
    Ok(Target::Cedar(parts))
}

/// The principal is one of these agents.
fn agent_is(ids: &[String]) -> String {
    let eqs = ids.iter().map(|a| format!("principal == Agent::{}", q(a))).collect::<Vec<_>>().join(" || ");
    format!("(principal is Agent && ({eqs}))")
}

/// A role as principals carry it: baseline and platform roles by name, an
/// org's own roles as `<org>/<role>`.
fn role_name(org: &str, r: &str) -> String {
    if BASELINE_ROLES.contains(&r) || r == "SuperAdmin" || r == "SecurityAdmin" || r.contains('/') {
        r.to_string()
    } else {
        format!("{org}/{r}")
    }
}

fn who_cond(org: &str, who: &Option<Who>) -> Result<(Option<String>, Vec<String>), String> {
    let Some(w) = who else { return Ok((None, Vec::new())) };
    let roles: Vec<String> = w.roles.iter().map(|r| role_name(org, r)).collect();
    let mut ors = Vec::new();
    if !roles.is_empty() {
        ors.push(format!("(principal has roles && principal.roles.containsAny({}))", set(&roles)));
    }
    match &w.agents {
        None => {}
        Some(AgentsSel::Any(s)) if s == "any" => ors.push("principal is Agent".to_string()),
        Some(AgentsSel::Any(s)) => return Err(format!("who.agents is \"any\" or a list of agent ids, not {s:?}")),
        Some(AgentsSel::Named(ids)) if !ids.is_empty() => ors.push(agent_is(ids)),
        Some(AgentsSel::Named(_)) => {}
    }
    if ors.is_empty() {
        return Ok((None, roles));
    }
    Ok((Some(format!("({})", ors.join(" || "))), roles))
}

fn scope_cond(full_name: &str, applies: &Option<AppliesTo>) -> (Option<String>, Placement) {
    let referenced = format!(
        "(principal has template && principal.template has policyRef && principal.template.policyRef.contains({}))",
        q(full_name));
    let Some(a) = applies else {
        // No applies_to: only where a flow or agent references it.
        return (Some(referenced), Placement::Org);
    };
    let mut ors = vec![referenced];
    if !a.tags.is_empty() {
        ors.push(format!("(principal has template && principal.template has tags && principal.template.tags.containsAny({}))", set(&a.tags)));
    }
    if !a.flows.is_empty() {
        ors.push(format!("(principal has template && principal.template has flow && {}.contains(principal.template.flow))", set(&a.flows)));
    }
    if !a.agents.is_empty() {
        ors.push(agent_is(&a.agents));
        ors.push(format!("(principal has template && principal.template has agent && {}.contains(principal.template.agent))", set(&a.agents)));
    }
    let selective = !(a.tags.is_empty() && a.flows.is_empty() && a.agents.is_empty());
    let placement = if a.org || a.projects.is_empty() { Placement::Org } else { Placement::Projects(a.projects.clone()) };
    // Placed on whole projects (or the org) with no narrower selector: every call there.
    if !selective && (a.org || !a.projects.is_empty()) {
        return (None, placement);
    }
    (Some(format!("({})", ors.join(" || "))), placement)
}

fn on_cond(on: &Option<On>) -> Result<Vec<String>, String> {
    let Some(on) = on else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    if !on.tools.is_empty() {
        let alts = on.tools.iter().map(|g| like(g).map(|p| format!("resource.name like {p}"))).collect::<Result<Vec<_>, _>>()?;
        out.push(format!("(resource is Tool && resource has name && ({}))", alts.join(" || ")));
    }
    if let Some(m) = &on.models {
        if !m.is_in.is_empty() && !m.not_in.is_empty() {
            return Err("on.models takes `in` or `not_in`, not both".into());
        }
        if !m.is_in.is_empty() {
            out.push(format!("(resource is Llm && resource has name && {}.contains(resource.name))", set(&m.is_in)));
        }
        if !m.not_in.is_empty() {
            out.push(format!("(resource is Llm && !(resource has name && {}.contains(resource.name)))", set(&m.not_in)));
        }
    }
    Ok(out)
}

/// Parse and validate against the A3T schema, as the engine will when it loads.
fn validate(src: &str) -> Result<(), String> {
    let set = cedar_policy::PolicySet::from_str(src).map_err(|e| format!("Cedar does not parse: {e}"))?;
    let (schema, _) = cedar_policy::Schema::from_cedarschema_str(crate::A3T_SCHEMA).map_err(|e| format!("A3T schema: {e}"))?;
    let result = cedar_policy::Validator::new(schema).validate(&set, cedar_policy::ValidationMode::default());
    if !result.validation_passed() {
        let errs: Vec<String> = result.validation_errors().map(|e| e.to_string()).collect();
        return Err(format!("Cedar does not fit the A3T schema: {}", errs.join("; ")));
    }
    Ok(())
}

fn annotate(id: &str, reason: &str) -> String {
    format!("@id({})\n@reason({})\n", q(id), q(reason))
}

/// An agent's own policy (its security profile): it applies to that agent
/// only, wherever the agent runs, so the file names no `applies_to`. Compiled
/// under the agent's project; the scope is the agent's id, so a copy of the
/// agent (clone, fork, sale) carries the same limits under its new id.
pub fn compile_agent_policy(project: &str, agent_id: &str, yaml: &str) -> Result<Compiled, String> {
    let mut f = PolicyFile::parse(yaml)?;
    if f.applies_to.is_some() {
        return Err("an agent's own policy applies to the agent itself: leave out applies_to".into());
    }
    f.applies_to = Some(AppliesTo { agents: vec![agent_id.to_string()], ..Default::default() });
    for r in f.role_ceiling.iter().flatten() {
        if !(BASELINE_ROLES.contains(&r.as_str()) || r.contains('/')) {
            return Err(format!("role_ceiling names {r:?}: a baseline role ({}) or an org role as <org>/<role>", BASELINE_ROLES.join(", ")));
        }
    }
    if let Some(v) = f.pii.as_deref().filter(|v| !matches!(*v, "redact" | "keep")) {
        return Err(format!("pii is {v:?}: redact or keep"));
    }
    // A ceiling or references alone are a whole policy: nothing for the engine.
    let no_rules = f.rules.is_empty() && f.cedar.as_deref().map_or(true, |c| c.trim().is_empty());
    if no_rules && (f.role_ceiling.is_some() || !f.uses.is_empty() || f.egress.is_some() || f.budget.is_some() || f.pii.is_some()) {
        if f.kind != "a3t/policy" {
            return Err(format!("kind is {:?}, not a3t/policy", f.kind));
        }
        if let Some(bad) = f.uses.iter().find(|u| u.split_once('/').is_none_or(|(o, n)| o.is_empty() || !valid_name(n))) {
            return Err(format!("uses: {bad:?} is not <org>/<policy name>"));
        }
        return Ok(Compiled { name: format!("{project}/{}", f.name), cedar: String::new(), placement: Placement::Org, hidden_apps: Vec::new() });
    }
    compile_file(project, f)
}

/// Compile a policy file of `org`.
pub fn compile_policy(org: &str, yaml: &str) -> Result<Compiled, String> {
    compile_file(org, PolicyFile::parse(yaml)?)
}

fn compile_file(org: &str, f: PolicyFile) -> Result<Compiled, String> {
    let own = f.applies_to.as_ref().is_some_and(|a| a.agents.len() == 1 && !a.org && a.projects.is_empty());
    if f.role_ceiling.is_some() && !own {
        return Err("role_ceiling belongs in an agent's own policy (its Security tab), not an org file".into());
    }
    if f.budget.is_some() && !own {
        return Err("budget belongs in an agent's own policy".into());
    }
    if f.pii.is_some() && !own {
        return Err("pii belongs in an agent's own policy".into());
    }
    if f.egress.is_some() && !own {
        return Err("egress belongs in an agent's own policy; an org's hosts are an egress/ file".into());
    }
    if !f.uses.is_empty() && !own {
        return Err("uses belongs in an agent's own policy; a flow names its policies in metadata.a3t.policyRef".into());
    }
    if let Some(bad) = f.uses.iter().find(|u| u.split_once('/').is_none_or(|(o, n)| o.is_empty() || !valid_name(n))) {
        return Err(format!("uses: {bad:?} is not <org>/<policy name>"));
    }
    if f.kind != "a3t/policy" {
        return Err(format!("kind is {:?}, not a3t/policy", f.kind));
    }
    if !valid_name(&f.name) {
        return Err(format!("name {:?}: lowercase letters, digits and '-'", f.name));
    }
    if f.rules.is_empty() && f.cedar.as_deref().map_or(true, |c| c.trim().is_empty()) {
        return Err("a policy needs at least one rule (or an expert cedar block)".into());
    }
    let cat = catalogue();
    let full = format!("{org}/{}", f.name);
    let (scope, placement) = scope_cond(&full, &f.applies_to);
    let mut out = String::new();
    let mut hidden = Vec::new();
    for (i, r) in f.rules.iter().enumerate() {
        let approval = !r.needs_approval.is_empty();
        match (r.cannot.is_empty(), approval) {
            (true, false) => return Err(format!("rule {}: `cannot` names nothing", i + 1)),
            (false, true) => return Err(format!("rule {}: a rule has `cannot` or `needs_approval`, not both", i + 1)),
            _ => {}
        }
        if r.timeout.is_some() && !approval {
            return Err(format!("rule {}: `timeout` goes with `needs_approval`", i + 1));
        }
        if r.reason.trim().is_empty() {
            return Err(format!("rule {}: give a reason (people see it when they are refused)", i + 1));
        }
        let timeout = match &r.timeout {
            Some(t) => Some(flowr_policy::policy_file::approval_timeout_secs(t).map_err(|e| format!("rule {}: {e}", i + 1))?),
            None => None,
        };
        let (who, roles) = who_cond(org, &r.who)?;
        // An approval is asked of the person behind an agent: it binds agents
        // only (a person calling a tool themselves is the approver).
        let who = if approval {
            Some(match who { Some(w) => format!("(principal is Agent && {w})"), None => "principal is Agent".to_string() })
        } else {
            who
        };
        let on = on_cond(&r.on)?;
        let items = if approval { &r.needs_approval } else { &r.cannot };
        for (k, item) in items.iter().enumerate() {
            match target_of(cat, item).map_err(|e| format!("rule {}: {e}", i + 1))? {
                Target::App(_) if approval => return Err(format!("rule {}: {item} is a console app; approvals are for what agents call", i + 1)),
                Target::App(app) => hidden.push(HiddenApp { app, roles: roles.clone() }),
                Target::Cedar(parts) => {
                    for (j, (head, res)) in parts.into_iter().enumerate() {
                        let conds: Vec<String> = scope.iter().cloned().chain(who.iter().cloned()).chain(res).chain(on.iter().cloned()).collect();
                        let when = if conds.is_empty() { String::new() } else { format!(" when {{\n  {}\n}}", conds.join(" &&\n  ")) };
                        out.push_str(&annotate(&format!("{full}/r{}.{}.{}", i + 1, k + 1, j + 1), &r.reason));
                        if approval {
                            // Kept out of the deciding set by the hook: a match
                            // asks a person instead of refusing (GOV-8).
                            out.push_str(&format!("@approval({})\n", q(&timeout.map(|t| t.to_string()).unwrap_or_default())));
                        }
                        out.push_str(&format!("forbid(principal, {head}, resource){when};\n\n"));
                    }
                }
            }
        }
    }
    if let Some(src) = f.cedar.as_deref().filter(|c| !c.trim().is_empty()) {
        out.push_str(&expert_cedar(&full, src)?);
    }
    if out.trim().is_empty() {
        // Only console rules (apps): nothing for the engine.
        return Ok(Compiled { name: full, cedar: out, placement, hidden_apps: hidden });
    }
    // A broken file never reaches the engine: it is refused here, at save.
    validate(&out)?;
    Ok(Compiled { name: full, cedar: out, placement, hidden_apps: hidden })
}

/// Check an expert block: Cedar that parses, `forbid` only, each statement named.
fn expert_cedar(full: &str, src: &str) -> Result<String, String> {
    let set = cedar_policy::PolicySet::from_str(src).map_err(|e| format!("cedar block: {e}"))?;
    if set.templates().next().is_some() {
        return Err("cedar block: templates are not allowed".into());
    }
    let mut n = 0;
    for p in set.policies() {
        n += 1;
        if p.effect() != cedar_policy::Effect::Forbid {
            return Err("cedar block: only `forbid` statements; org policy can narrow access, never widen it".into());
        }
    }
    if n == 0 {
        return Err("cedar block: no statements".into());
    }
    // Statements without an @id get one under this policy's name, so every
    // decision names the file that made it.
    let mut out = String::new();
    for (i, p) in set.policies().enumerate() {
        let text = p.to_string();
        if p.annotation("id").is_some() {
            out.push_str(&text);
        } else {
            out.push_str(&format!("@id({})\n{text}", q(&format!("{full}/cedar.{}", i + 1))));
        }
        out.push_str("\n\n");
    }
    Ok(out)
}

/// Compile a role file of `org`: forbids for what the role removes from its base.
pub fn compile_role(org: &str, yaml: &str) -> Result<Compiled, String> {
    let f: RoleFile = serde_yaml::from_str(yaml).map_err(|e| format!("role file: {e}"))?;
    if f.kind != "a3t/role" {
        return Err(format!("kind is {:?}, not a3t/role", f.kind));
    }
    if !valid_name(&f.name) {
        return Err(format!("name {:?}: lowercase letters, digits and '-'", f.name));
    }
    if !BASELINE_ROLES.contains(&f.extends.as_str()) {
        return Err(format!("extends {:?}: one of {}", f.extends, BASELINE_ROLES.join(", ")));
    }
    let cat = catalogue();
    let full = format!("{org}/{}", f.name);
    let role = format!("{org}/{}", f.name);
    let holder = format!("principal has roles && principal.roles.contains({})", q(&role));
    let mut out = String::new();
    let mut hidden = Vec::new();
    for (k, item) in f.remove.iter().enumerate() {
        match target_of(cat, item).map_err(|e| format!("remove: {e}"))? {
            Target::App(app) => hidden.push(HiddenApp { app, roles: vec![role.clone()] }),
            Target::Cedar(parts) => {
                for (j, (head, res)) in parts.into_iter().enumerate() {
                    let conds: Vec<String> = std::iter::once(holder.clone()).chain(res).collect();
                    out.push_str(&annotate(&format!("{full}/remove.{}.{}", k + 1, j + 1),
                        &format!("The {} role does not include this ({item}).", f.name)));
                    out.push_str(&format!("forbid(principal, {head}, resource) when {{\n  {}\n}};\n\n", conds.join(" &&\n  ")));
                }
            }
        }
    }
    if !out.is_empty() {
        validate(&out)?;
    }
    Ok(Compiled { name: full, cedar: out, placement: Placement::Org, hidden_apps: hidden })
}

// ── An org's whole set of files ──────────────────────────────────────────────

/// One file of an org's policy repo: its path in the repo (below the folder
/// A3T reads) and its text.
#[derive(Debug, Clone)]
pub struct OrgFile {
    pub path: String,
    pub content: String,
}

/// An org's files, compiled: the Cedar each project's policy set carries, the
/// console apps hidden per role, and who holds which role where.
#[derive(Debug, Clone, Default, Serialize)]
pub struct OrgSet {
    /// `(project, policy name, cedar)`, ready for the project's policy set.
    pub policies: Vec<(String, String, String)>,
    pub hidden_apps: Vec<HiddenApp>,
    pub access: Vec<AccessFile>,
    /// What each file became: `{path, kind, name, placement}` (no Cedar).
    pub files: Vec<serde_json::Value>,
}

/// Compile every file of `org` at once. `projects` are the org's projects:
/// nothing in the files can reach a project outside them. All or nothing: any
/// error in any file refuses the set (each error names its file), so a broken
/// push never half-applies.
pub fn compile_org_set(org: &str, projects: &[String], files: &[OrgFile]) -> Result<OrgSet, Vec<String>> {
    let mut errors = Vec::new();
    let mut set = OrgSet::default();
    let mut names = std::collections::BTreeSet::new();
    let mut roles: Vec<String> = Vec::new();
    let mut access_files = Vec::new();
    // Roles the org's rules bind (`who.roles`), checked once every role file is known.
    let mut named_roles: Vec<(String, usize, String)> = Vec::new();
    let place = |c: &Compiled, path: &str, errors: &mut Vec<String>| -> Vec<String> {
        match &c.placement {
            Placement::Org => projects.to_vec(),
            Placement::Projects(ps) => {
                let outside: Vec<&String> = ps.iter().filter(|p| !projects.contains(p)).collect();
                if !outside.is_empty() {
                    errors.push(format!("{path}: applies_to names projects that are not {org}'s: {}",
                        outside.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
                    return Vec::new();
                }
                ps.clone()
            }
        }
    };
    for f in files {
        let Some(kind) = kind_of_path(&f.path) else { continue };
        if kind == "policy" {
            if let Ok(p) = PolicyFile::parse(&f.content) {
                for (i, r) in p.rules.iter().enumerate() {
                    for role in r.who.iter().flat_map(|w| w.roles.iter()) {
                        named_roles.push((f.path.clone(), i + 1, role.clone()));
                    }
                }
            }
        }
        if kind == "egress" {
            // RT-1: where the org's flows may call out to; nothing for the engine.
            match flowr_policy::policy_file::EgressFile::parse(&f.content) {
                Ok(_) => set.files.push(serde_json::json!({ "path": f.path, "kind": "egress", "name": f.path, "placement": "org" })),
                Err(e) => errors.push(format!("{}: {e}", f.path)),
            }
            continue;
        }
        if kind == "org-access" {
            // GOV-5: the org's sign-in mapping; nothing for the engine.
            match flowr_policy::policy_file::OrgAccessFile::parse(&f.content) {
                Ok(_) => set.files.push(serde_json::json!({ "path": f.path, "kind": "org-access", "name": "org", "placement": "org" })),
                Err(e) => errors.push(format!("{}: {e}", f.path)),
            }
            continue;
        }
        let compiled = match kind {
            "policy" => compile_policy(org, &f.content),
            "role" => compile_role(org, &f.content),
            _ => {
                access_files.push(f);
                continue;
            }
        };
        let c = match compiled {
            Ok(c) => c,
            Err(e) => {
                errors.push(format!("{}: {e}", f.path));
                continue;
            }
        };
        if !names.insert((kind, c.name.clone())) {
            errors.push(format!("{}: another {kind} file is also named {:?}", f.path, c.name));
            continue;
        }
        if kind == "role" {
            roles.push(c.name.rsplit('/').next().unwrap_or_default().to_string());
        }
        let short = c.name.rsplit('/').next().unwrap_or_default();
        // Dots: an org slug never has one, so one org's labels never prefix another's.
        let label = format!("org.{org}.{kind}.{short}");
        for p in place(&c, &f.path, &mut errors) {
            if !c.cedar.trim().is_empty() {
                set.policies.push((p, label.clone(), c.cedar.clone()));
            }
        }
        set.hidden_apps.extend(c.hidden_apps.iter().cloned());
        set.files.push(serde_json::json!({ "path": f.path, "kind": kind, "name": c.name, "placement": c.placement }));
    }
    // A rule must bind a role that exists: one it misnames would bind nobody,
    // silently. A baseline role, a platform role, or one of the org's roles/ files.
    for (path, rule, role) in named_roles {
        let own = role.strip_prefix(&format!("{org}/")).unwrap_or(&role);
        let known = BASELINE_ROLES.contains(&role.as_str()) || role == "SuperAdmin" || role == "SecurityAdmin"
            || (!own.contains('/') && roles.iter().any(|r| r == own));
        if !known {
            errors.push(format!("{path}: rule {rule}: who.roles names {role:?}: not a baseline role ({}), SuperAdmin, SecurityAdmin or one of the org's roles/ files",
                BASELINE_ROLES.join(", ")));
        }
    }
    // Access files last: they may name the org's own roles.
    let mut mapped = std::collections::BTreeSet::new();
    for f in access_files {
        let a = match AccessFile::parse(&f.content) {
            Ok(a) => a,
            Err(e) => {
                errors.push(format!("{}: {e}", f.path));
                continue;
            }
        };
        if !projects.contains(&a.project) {
            errors.push(format!("{}: project {:?} is not {org}'s", f.path, a.project));
            continue;
        }
        if !mapped.insert(a.project.clone()) {
            errors.push(format!("{}: another access file already maps project {:?}", f.path, a.project));
            continue;
        }
        for (who, role) in a.groups.iter().chain(a.users.iter()) {
            if !BASELINE_ROLES.contains(&role.as_str()) && !roles.contains(role) {
                errors.push(format!("{}: {who} → {role:?}: not a baseline role ({}) or one of the org's roles/ files",
                    f.path, BASELINE_ROLES.join(", ")));
            }
        }
        set.files.push(serde_json::json!({ "path": f.path, "kind": "access", "name": a.project, "placement": { "projects": [a.project] } }));
        set.access.push(a);
    }
    if errors.is_empty() { Ok(set) } else { Err(errors) }
}
