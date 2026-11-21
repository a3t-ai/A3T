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

pub use flowr_policy::policy_file::{kind_of_path, AccessFile, AgentsSel, AppliesTo, Cond, InOrNot, NetworkFile, On, PolicyFile, RoleFile, Rule, Who};

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

/// Who a rule binds. A baseline role binds that exact level where the call
/// lands (`principal.level`): roles stack (an Admin also holds Operator and
/// Viewer), and "Viewers" means Viewers. Platform roles and the org's own
/// roles bind whoever holds them. `agents: acting` narrows the roles to agents
/// acting for them; `people_only` to people calling themselves.
fn who_cond(org: &str, who: &Option<Who>) -> Result<(Option<String>, Vec<String>), String> {
    let Some(w) = who else { return Ok((None, Vec::new())) };
    let roles: Vec<String> = w.roles.iter().map(|r| role_name(org, r)).collect();
    let levels: Vec<String> = w.roles.iter().filter(|r| BASELINE_ROLES.contains(&r.as_str())).cloned().collect();
    let held: Vec<String> = w.roles.iter().filter(|r| !BASELINE_ROLES.contains(&r.as_str())).map(|r| role_name(org, r)).collect();
    let mut by_role = Vec::new();
    if !levels.is_empty() {
        by_role.push(format!("(principal has level && {}.contains(principal.level))", set(&levels)));
    }
    if !held.is_empty() {
        by_role.push(format!("(principal has roles && principal.roles.containsAny({}))", set(&held)));
    }
    let role_expr = (!by_role.is_empty()).then(|| if by_role.len() == 1 { by_role[0].clone() } else { format!("({})", by_role.join(" || ")) });
    if w.people_only && w.agents.is_some() {
        return Err("who: people_only and agents can't go together".into());
    }
    let expr = match &w.agents {
        None if w.people_only => Some(match &role_expr { Some(r) => format!("(principal is User && {r})"), None => "principal is User".to_string() }),
        None => role_expr,
        Some(AgentsSel::Any(s)) if s == "any" => Some(match &role_expr { Some(r) => format!("({r} || principal is Agent)"), None => "principal is Agent".to_string() }),
        Some(AgentsSel::Any(s)) if s == "acting" => match &role_expr {
            Some(r) => Some(format!("(principal is Agent && {r})")),
            None => return Err("who.agents: acting needs roles (the agents acting for them)".into()),
        },
        Some(AgentsSel::Any(s)) => return Err(format!("who.agents is \"any\", \"acting\" or a list of agent ids, not {s:?}")),
        Some(AgentsSel::Named(ids)) if !ids.is_empty() => Some(match &role_expr { Some(r) => format!("({r} || {})", agent_is(ids)), None => agent_is(ids) }),
        Some(AgentsSel::Named(_)) => role_expr,
    };
    Ok((expr, roles))
}

/// Named networks the org's rules can name: name → address ranges.
pub type Networks = std::collections::BTreeMap<String, Vec<String>>;

/// What a condition becomes: a Cedar expression, or a count A3T keeps (rate,
/// lockout, first use, tokens), checked by the hook when the rest matches.
enum CondOut {
    Expr(String),
    Count(serde_json::Value),
}

fn num(c: &Cond, k: &str, min: u64, max: u64) -> Result<u64, String> {
    let v = c.get(k).ok_or_else(|| format!("{}: give {k}", c.kind))?;
    let n: u64 = v.trim().parse().map_err(|_| format!("{}.{k}: a whole number, not {v:?}", c.kind))?;
    if !(min..=max).contains(&n) {
        return Err(format!("{}.{k}: between {min} and {max}", c.kind));
    }
    Ok(n)
}

fn hhmm(c: &Cond, k: &str) -> Result<u64, String> {
    let v = c.get(k).ok_or_else(|| format!("{}: give {k} (HH:MM)", c.kind))?;
    let (h, m) = v.trim().split_once(':').ok_or_else(|| format!("{}.{k}: HH:MM, not {v:?}", c.kind))?;
    match (h.parse::<u64>(), m.parse::<u64>()) {
        (Ok(h), Ok(m)) if h <= 24 && m < 60 && h * 60 + m <= 1440 => Ok(h * 60 + m),
        _ => Err(format!("{}.{k}: HH:MM, not {v:?}", c.kind)),
    }
}

/// `YYYY-MM-DD` → YYYYMMDD.
pub fn day_number(v: &str) -> Option<i64> {
    let p: Vec<&str> = v.trim().split('-').collect();
    if p.len() != 3 || p[0].len() != 4 || p[1].len() != 2 || p[2].len() != 2 {
        return None;
    }
    let (y, m, d) = (p[0].parse::<i64>().ok()?, p[1].parse::<i64>().ok()?, p[2].parse::<i64>().ok()?);
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some(y * 10_000 + m * 100 + d)
}

fn date(c: &Cond, k: &str) -> Result<i64, String> {
    let v = c.get(k).ok_or_else(|| format!("{}: give {k} (YYYY-MM-DD)", c.kind))?;
    day_number(v).ok_or_else(|| format!("{}.{k}: YYYY-MM-DD, not {v:?}", c.kind))
}

fn list_of(c: &Cond, k: &str) -> Result<Vec<String>, String> {
    let v: Vec<String> = c.get(k).unwrap_or("").split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
    if v.is_empty() {
        return Err(format!("{}: give {k}", c.kind));
    }
    Ok(v)
}

fn plain(c: &Cond, k: &str) -> Result<String, String> {
    let v = c.get(k).map(str::trim).unwrap_or("");
    if v.is_empty() || v.contains('"') || v.contains('\\') {
        return Err(format!("{}: give {k}", c.kind));
    }
    Ok(v.to_string())
}

const DAY_NAMES: &[&str] = &["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

fn weekdays(v: &str) -> Result<Vec<i64>, String> {
    let idx = |d: &str| DAY_NAMES.iter().position(|x| *x == d).map(|i| i as i64 + 1);
    let mut out = Vec::new();
    for part in v.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (idx(a).ok_or(format!("hours.days: {a:?} is not a day (mon … sun)"))?, idx(b).ok_or(format!("hours.days: {b:?} is not a day (mon … sun)"))?);
                if a > b {
                    return Err(format!("hours.days: {part:?} runs backwards"));
                }
                out.extend(a..=b);
            }
            None => out.push(idx(part).ok_or(format!("hours.days: {part:?} is not a day (mon … sun)"))?),
        }
    }
    if out.is_empty() {
        return Err("hours.days: say which days (mon-fri, sat-sun, mon,wed)".into());
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// The condition kinds a rule may use (the console's catalogue).
pub const COND_KINDS: &[&str] = &["hours", "dates", "after_deploy", "rate", "refusals", "first_use", "tokens", "ip", "network", "via",
    "group", "person", "mfa", "org_idp", "session_age", "acting_as", "tools", "models_in", "models_not_in", "destructive", "untrusted", "pii", "arg_over"];
pub const CHANNELS: &[&str] = &["console", "vscode", "mcp", "api-key", "trigger"];

fn cond(c: &Cond, networks: &Networks) -> Result<CondOut, String> {
    use CondOut::*;
    let known = |ks: &[&str]| -> Result<(), String> {
        match c.params.keys().find(|k| !ks.contains(&k.as_str())) {
            Some(k) => Err(format!("{}: unknown parameter {k:?}{}", c.kind, if k == "tz" { " (times are in the org's time zone, set under Settings)" } else { "" })),
            None => Ok(()),
        }
    };
    Ok(match c.kind.as_str() {
        "hours" => {
            known(&["days", "from", "to"])?;
            let days = weekdays(c.get("days").unwrap_or("mon-sun"))?;
            let (from, to) = (hhmm(c, "from")?, hhmm(c, "to")?);
            if from == to {
                return Err("hours: from and to are the same".into());
            }
            let mins = if from < to {
                format!("context.local.minute >= {from} && context.local.minute < {to}")
            } else {
                // Over midnight: 22:00–06:00.
                format!("(context.local.minute >= {from} || context.local.minute < {to})")
            };
            let ds = format!("[{}]", days.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", "));
            Expr(format!("(context has local && {ds}.contains(context.local.weekday) && {mins})"))
        }
        "dates" => {
            known(&["from", "to"])?;
            let (a, b) = (date(c, "from")?, date(c, "to")?);
            if a > b {
                return Err("dates: from is after to".into());
            }
            Expr(format!("(context has local && context.local.day >= {a} && context.local.day <= {b})"))
        }
        "after_deploy" => {
            known(&["min"])?;
            Expr(format!("(context has since_deploy_s && context.since_deploy_s < {})", num(c, "min", 1, 10_080)? * 60))
        }
        "rate" => {
            known(&["n", "per", "by"])?;
            let n = num(c, "n", 1, 1_000_000)?;
            let per = c.get("per").unwrap_or("hour");
            if !["minute", "hour", "day"].contains(&per) {
                return Err(format!("rate.per: minute, hour or day, not {per:?}"));
            }
            let by = c.get("by").unwrap_or("person");
            if !["person", "agent", "project", "org"].contains(&by) {
                return Err(format!("rate.by: person, agent, project or org, not {by:?}"));
            }
            Count(serde_json::json!({ "kind": "rate", "n": n, "per": per, "by": by }))
        }
        "refusals" => {
            known(&["n", "mins", "block"])?;
            Count(serde_json::json!({ "kind": "refusals", "n": num(c, "n", 1, 1000)?, "mins": num(c, "mins", 1, 1440)?, "block": num(c, "block", 1, 10_080)? }))
        }
        "first_use" => {
            known(&[])?;
            Count(serde_json::json!({ "kind": "first_use" }))
        }
        "tokens" => {
            known(&["n"])?;
            Count(serde_json::json!({ "kind": "tokens", "n": num(c, "n", 1, 1_000_000_000_000)? }))
        }
        "ip" => {
            known(&["cidr"])?;
            let r = plain(c, "cidr")?;
            if !flowr_policy::policy_file::valid_range(&r) {
                return Err(format!("ip.cidr: an IP address or a CIDR range, not {r:?}"));
            }
            Expr(format!("(context has ip && context.ip.isInRange(ip({})))", q(&r)))
        }
        "network" => {
            known(&["name"])?;
            let n = plain(c, "name")?;
            let rs = networks.get(&n).ok_or_else(|| format!("network: no networks/ file is named {n:?}"))?;
            let alts = rs.iter().map(|r| format!("context.ip.isInRange(ip({}))", q(r))).collect::<Vec<_>>().join(" || ");
            Expr(format!("(context has ip && ({alts}))"))
        }
        "via" => {
            known(&["via"])?;
            let v = plain(c, "via")?;
            if !CHANNELS.contains(&v.as_str()) {
                return Err(format!("via: one of {}, not {v:?}", CHANNELS.join(", ")));
            }
            Expr(format!("(context has channel && context.channel == {})", q(&v)))
        }
        "group" => {
            known(&["name"])?;
            Expr(format!("(principal has groups && principal.groups.contains({}))", q(&plain(c, "name")?)))
        }
        "person" => {
            known(&["email"])?;
            let e = plain(c, "email")?;
            if !e.contains('@') {
                return Err(format!("person.email: an email address, not {e:?}"));
            }
            Expr(format!("(principal has email && principal.email == {})", q(&e.to_lowercase())))
        }
        "mfa" => { known(&[])?; Expr("(context has auth && context.auth.second_factor)".into()) }
        "org_idp" => { known(&[])?; Expr("(context has auth && context.auth.org_idp)".into()) }
        "session_age" => {
            known(&["h"])?;
            Expr(format!("(context has auth && context.auth has age_s && context.auth.age_s < {})", num(c, "h", 1, 720)? * 3600))
        }
        "acting_as" => { known(&[])?; Expr("(principal has impersonated_by)".into()) }
        "tools" => {
            known(&["tools"])?;
            let alts = list_of(c, "tools")?.iter().map(|g| like(g).map(|p| format!("resource.name like {p}"))).collect::<Result<Vec<_>, _>>()?;
            Expr(format!("(resource is Tool && resource has name && ({}))", alts.join(" || ")))
        }
        "models_in" => {
            known(&["models"])?;
            Expr(format!("(resource is Llm && resource has name && {}.contains(resource.name))", set(&list_of(c, "models")?)))
        }
        "models_not_in" => {
            known(&["models"])?;
            Expr(format!("(resource is Llm && !(resource has name && {}.contains(resource.name)))", set(&list_of(c, "models")?)))
        }
        "destructive" => { known(&[])?; Expr("(resource is Tool && resource has destructive && resource.destructive)".into()) }
        "untrusted" => { known(&[])?; Expr("(context has untrusted_seen && context.untrusted_seen)".into()) }
        "pii" => { known(&[])?; Expr("(context has args_pii && context.args_pii)".into()) }
        "arg_over" => {
            known(&["arg", "n"])?;
            let a = plain(c, "arg")?;
            if !a.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') || !a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(format!("arg_over.arg: an argument name (amount, max_rows), not {a:?}"));
            }
            let n: i64 = c.get("n").unwrap_or("").trim().parse().map_err(|_| "arg_over.n: a whole number".to_string())?;
            Expr(format!("(resource is Tool && resource.hasTag({}) && resource.getTag({}) > {n})", q(&a), q(&a)))
        }
        other => return Err(format!("{other:?} is not a condition ({})", COND_KINDS.join(", "))),
    })
}

/// Who answers an approval: a baseline or platform role, an org role, or `group:<name>`.
fn valid_approver(org: &str, a: &str) -> Result<String, String> {
    let a = a.trim();
    if BASELINE_ROLES.contains(&a) || a == "SuperAdmin" || a == "SecurityAdmin" || a == "admin" || a == "securityadmin" {
        return Ok(a.to_string());
    }
    if let Some(g) = a.strip_prefix("group:") {
        if !g.is_empty() && !g.contains('"') {
            return Ok(a.to_string());
        }
    }
    if valid_name(a) {
        return Ok(role_name(org, a));
    }
    Err(format!("approver {a:?}: a role (Admin, securityadmin, …) or group:<name>"))
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
    compile_file(project, f, &Networks::new())
}

/// Compile a policy file of `org` (one that names no networks).
pub fn compile_policy(org: &str, yaml: &str) -> Result<Compiled, String> {
    compile_file(org, PolicyFile::parse(yaml)?, &Networks::new())
}

/// Compile a policy file of `org` whose rules may name these networks.
pub fn compile_policy_with(org: &str, yaml: &str, networks: &Networks) -> Result<Compiled, String> {
    compile_file(org, PolicyFile::parse(yaml)?, networks)
}

fn compile_file(org: &str, f: PolicyFile, networks: &Networks) -> Result<Compiled, String> {
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
        let n = i + 1;
        let err = |e: String| format!("rule {n}: {e}");
        let approval = !r.needs_approval.is_empty();
        let only = !r.only.is_empty();
        match [!r.cannot.is_empty(), approval, only].iter().filter(|x| **x).count() {
            0 => return Err(err("say what it covers: `cannot`, `needs_approval` or `only`".into())),
            1 => {}
            _ => return Err(err("a rule has one of `cannot`, `needs_approval` or `only`".into())),
        }
        let otherwise_ask = match r.otherwise.as_deref() {
            None | Some("block") => false,
            Some("ask") => true,
            Some(o) => return Err(err(format!("otherwise is block or ask, not {o:?}"))),
        };
        if r.otherwise.is_some() && !only {
            return Err(err("`otherwise` goes with `only`".into()));
        }
        if only && r.allowed_only_when.is_empty() {
            return Err(err("an `only` rule needs `allowed_only_when` (the conditions)".into()));
        }
        if !only && !r.allowed_only_when.is_empty() {
            return Err(err("`allowed_only_when` goes with `only`".into()));
        }
        let ask = approval || (only && otherwise_ask);
        if r.timeout.is_some() && !ask {
            return Err(err("`timeout` goes with an approval (needs_approval, or otherwise: ask)".into()));
        }
        if r.approver.is_some() && !ask {
            return Err(err("`approver` goes with an approval (needs_approval, or otherwise: ask)".into()));
        }
        if r.log_only && ask {
            return Err(err("a rule that only logs doesn't ask anyone".into()));
        }
        if r.reason.trim().is_empty() {
            return Err(err("give a reason (people see it when they are refused)".into()));
        }
        let timeout = match &r.timeout {
            Some(t) => Some(flowr_policy::policy_file::approval_timeout_secs(t).map_err(err)?),
            None => None,
        };
        let approver = r.approver.as_deref().map(|a| valid_approver(org, a)).transpose().map_err(err)?;
        let until = match &r.until {
            Some(u) => Some(day_number(u).ok_or_else(|| err(format!("until: YYYY-MM-DD, not {u:?}")))?),
            None => None,
        };
        // Conditions: `when` (all hold), `allowed_only_when` (the only-window),
        // `unless` (exceptions). Counts only in a Block or Ask rule's `when`.
        let mut when_exprs = Vec::new();
        let mut counts = Vec::new();
        for c in &r.when {
            match cond(c, networks).map_err(err)? {
                CondOut::Expr(e) => when_exprs.push(e),
                CondOut::Count(v) if !only && !r.log_only => counts.push(v),
                CondOut::Count(_) => return Err(err(format!("{} is counted by A3T: use it in a Block or Ask rule's conditions", c.kind))),
            }
        }
        let plain_list = |cs: &[Cond], what: &str| -> Result<Vec<String>, String> {
            cs.iter().map(|c| match cond(c, networks)? {
                CondOut::Expr(e) => Ok(e),
                CondOut::Count(_) => Err(format!("{} is counted by A3T: it can't be {what}", c.kind)),
            }).collect::<Result<Vec<_>, _>>().map_err(err)
        };
        let window = plain_list(&r.allowed_only_when, "an only-when condition")?;
        let except = plain_list(&r.unless, "an exception")?;
        if let Some(d) = until {
            when_exprs.push(format!("(context has local && context.local.day <= {d})"));
        }
        let (who, roles) = who_cond(org, &r.who).map_err(err)?;
        let on = on_cond(&r.on).map_err(err)?;
        let items = if approval { &r.needs_approval } else if only { &r.only } else { &r.cannot };
        // Who each statement binds, and whether it asks: an approval is asked of
        // the person behind an agent, so it binds agents; a person calling
        // themselves outside an only-window is refused.
        let mut forms: Vec<(Option<String>, bool, &str)> = Vec::new();
        if ask {
            forms.push((Some(match &who { Some(w) => format!("(principal is Agent && {w})"), None => "principal is Agent".to_string() }), true, ""));
            if only {
                forms.push((Some(match &who { Some(w) => format!("(principal is User && {w})"), None => "principal is User".to_string() }), false, "p"));
            }
        } else {
            forms.push((who.clone(), false, ""));
        }
        let mut unless = Vec::new();
        if only {
            unless.push(window.join(" &&\n  "));
        }
        if !except.is_empty() {
            unless.push(except.join(" ||\n  "));
        }
        let unless_txt: String = unless.iter().map(|u| format!(" unless {{\n  {u}\n}}")).collect();
        for (k, item) in items.iter().enumerate() {
            match target_of(cat, item).map_err(err)? {
                Target::App(_) if ask => return Err(err(format!("{item} is a console app; approvals are for what agents call"))),
                Target::App(_) if only || !when_exprs.is_empty() || !except.is_empty() || !counts.is_empty() || r.log_only =>
                    return Err(err(format!("{item} is a console app: hide it with a plain `cannot` rule (no conditions)"))),
                Target::App(app) => hidden.push(HiddenApp { app, roles: roles.clone() }),
                Target::Cedar(parts) => {
                    for (j, (head, res)) in parts.into_iter().enumerate() {
                        for (form_who, form_ask, suffix) in &forms {
                            let conds: Vec<String> = scope.iter().cloned().chain(form_who.iter().cloned()).chain(res.iter().cloned())
                                .chain(on.iter().cloned()).chain(when_exprs.iter().cloned()).collect();
                            let when = if conds.is_empty() { String::new() } else { format!(" when {{\n  {}\n}}", conds.join(" &&\n  ")) };
                            let reason = if *suffix == "p" {
                                format!("{} (Outside these conditions a person's own call is refused; an agent's waits for approval.)", r.reason.trim())
                            } else {
                                r.reason.clone()
                            };
                            out.push_str(&annotate(&format!("{full}/r{n}.{}.{}{suffix}", k + 1, j + 1), &reason));
                            if *form_ask {
                                // Kept out of the deciding set by the hook: a match
                                // asks a person instead of refusing (GOV-8).
                                out.push_str(&format!("@approval({})\n", q(&timeout.map(|t| t.to_string()).unwrap_or_default())));
                                if let Some(a) = &approver {
                                    out.push_str(&format!("@approver({})\n", q(a)));
                                }
                            }
                            if !counts.is_empty() {
                                // Checked by the hook against what A3T counted.
                                out.push_str(&format!("@count({})\n", q(&serde_json::Value::Array(counts.clone()).to_string())));
                            }
                            if r.log_only {
                                out.push_str("@log(\"true\")\n");
                            }
                            // An org rule is the org's decision: the person an agent
                            // acts for can't wave it through as a delegation prompt.
                            out.push_str("@firm(\"true\")\n");
                            out.push_str(&format!("forbid(principal, {head}, resource){when}{unless_txt};\n\n"));
                        }
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
    /// What each file became: `{path, kind, name, placement, cedar}`.
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
    // Named networks first: policy rules may name them.
    let mut networks = Networks::new();
    for f in files.iter().filter(|f| kind_of_path(&f.path) == Some("network")) {
        match NetworkFile::parse(&f.content) {
            Ok(n) if networks.contains_key(&n.name) => errors.push(format!("{}: another networks/ file is also named {:?}", f.path, n.name)),
            Ok(n) => {
                set.files.push(serde_json::json!({ "path": f.path, "kind": "network", "name": n.name, "placement": "org" }));
                networks.insert(n.name, n.ranges.iter().map(|r| r.trim().to_string()).collect());
            }
            Err(e) => errors.push(format!("{}: {e}", f.path)),
        }
    }
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
        if kind == "network" {
            continue;
        }
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
            "policy" => compile_policy_with(org, &f.content, &networks),
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
        // The Cedar it became, for people who check what a rule means.
        set.files.push(serde_json::json!({ "path": f.path, "kind": kind, "name": c.name, "placement": c.placement, "cedar": c.cedar }));
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
