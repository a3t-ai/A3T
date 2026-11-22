//! Org policy and role files (`kind: a3t/policy`, `kind: a3t/role`): their
//! shape, and how to say what a rule does in plain words. The Cedar compiler is
//! EE (`flowr-aesop::policy_files`); this part is shared so the console and CE
//! routes can show a policy without Cedar.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyFile {
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Where it applies. Absent: only where a flow or agent references it.
    #[serde(default)]
    pub applies_to: Option<AppliesTo>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Expert Cedar (`forbid` statements only).
    #[serde(default)]
    pub cedar: Option<String>,
    /// An agent's own policy only (UI-3): the most authority the agent carries,
    /// whoever runs it — its roles are the caller's roles that are also here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_ceiling: Option<Vec<String>>,
    /// An agent's own policy only (GOV-6): org policies it runs under
    /// (`<org>/<name>`), as a flow names them in `metadata.a3t.policyRef`. A
    /// policy with no `applies_to` binds only where it is named like this.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<String>,
    /// An agent's own policy only (RT-1): the hosts it may call out to — within
    /// what its org's egress files allow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress: Option<Vec<String>>,
    /// An agent's own policy only (RT-4): what it may spend; over it, it stops
    /// until the next day (UTC) and an alert goes to the decision log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<Budget>,
    /// An agent's own policy only (RT-10): `redact` keeps personal data and
    /// secrets out of what the agent stores — its transcripts, its memory and
    /// what its memory tools are given (`keep`, the default, stores them as
    /// said). The decision log and FlowR's logs are redacted either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pii: Option<String>,
}

/// An agent's budget (RT-4). Each limit is optional.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    /// Turns (runs) a day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_per_day: Option<u64>,
    /// Model tokens (prompt + completion) a day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_per_day: Option<u64>,
    /// Calls a minute through each of its keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_calls_per_minute: Option<u64>,
}

/// Where an org's flows and agents may call out to (`egress/<name>.yaml`, RT-1).
/// With one enforced, only `hosts` are reached (several files: their union);
/// `private` names hosts, IPs or CIDRs on the org's own network that may be
/// reached although private (never cloud metadata unless named).
///
/// ```yaml
/// kind: a3t/egress
/// hosts: [api.github.com, "*.slack.com"]
/// private: [mcp.internal.acme.com, 10.20.0.0/16]
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EgressFile {
    pub kind: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default)]
    pub private: Vec<String>,
}

impl EgressFile {
    pub fn parse(yaml: &str) -> Result<Self, String> {
        let f: Self = serde_yaml::from_str(yaml).map_err(|e| format!("not an egress file: {e}"))?;
        if f.kind != "a3t/egress" {
            return Err(format!("kind is {:?}, not a3t/egress", f.kind));
        }
        for h in f.hosts.iter().chain(f.private.iter()) {
            let h = h.trim();
            if h.is_empty() || h.contains("://") || h.contains(' ') || h.contains('/') && h.split_once('/').is_none_or(|(n, b)| n.parse::<std::net::IpAddr>().is_err() || b.parse::<u8>().is_err()) {
                return Err(format!("{h:?}: a host name (api.github.com, *.slack.com), an IP or a CIDR range"));
            }
        }
        Ok(f)
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppliesTo {
    /// Every project in the org.
    #[serde(default)]
    pub org: bool,
    #[serde(default)]
    pub projects: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// `<project>/<flow>`.
    #[serde(default)]
    pub flows: Vec<String>,
    #[serde(default)]
    pub agents: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    #[serde(default)]
    pub who: Option<Who>,
    /// Catalogue ids (`flows.run`, `apps.security`) or tool globs (`tools.delete_*`).
    #[serde(default)]
    pub cannot: Vec<String>,
    /// What agents may do only once a person approves that call (GOV-8): the
    /// same ids and globs as `cannot`. A rule has `cannot` or `needs_approval`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs_approval: Vec<String>,
    /// How long a `needs_approval` question may wait: `30m`, `4h`, `2d`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
    #[serde(default)]
    pub on: Option<On>,
    pub reason: String,
    /// "Allow only when": these (same ids and globs as `cannot`) are allowed
    /// only while every `allowed_only_when` condition holds; `otherwise` says
    /// what happens the rest of the time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_only_when: Vec<Cond>,
    /// For an `only` rule: `block` (the default) or `ask` (a person approves,
    /// within `timeout`, asked of `approver`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otherwise: Option<String>,
    /// More conditions, all of which must hold for the rule to apply.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<Cond>,
    /// Exceptions: the rule doesn't apply when any of these holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unless: Vec<Cond>,
    /// The last day the rule applies (`YYYY-MM-DD`, in the org's time zone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Who answers an approval: a role (`Admin`), an org role
    /// (`securityadmin`) or a group (`group:finance`). Absent: the person the
    /// agent acts for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approver: Option<String>,
    /// Only logs what it would refuse (as `policy:shadow`); never refuses.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub log_only: bool,
}

/// A rule condition: a bare name (`mfa`) or a name with its parameters
/// (`hours: {days: mon-fri, from: "09:00", to: "18:00"}`). Parameters are kept
/// as strings; the compiler checks each kind's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cond {
    pub kind: String,
    pub params: std::collections::BTreeMap<String, String>,
}

impl Cond {
    pub fn get(&self, k: &str) -> Option<&str> {
        self.params.get(k).map(|s| s.as_str())
    }
}

impl<'de> Deserialize<'de> for Cond {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let v = serde_yaml::Value::deserialize(d)?;
        let scalar = |v: &serde_yaml::Value| -> Option<String> {
            match v {
                serde_yaml::Value::String(s) => Some(s.clone()),
                serde_yaml::Value::Number(n) => Some(n.to_string()),
                serde_yaml::Value::Bool(b) => Some(b.to_string()),
                serde_yaml::Value::Sequence(xs) => Some(xs.iter().filter_map(|x| match x {
                    serde_yaml::Value::String(s) => Some(s.clone()),
                    serde_yaml::Value::Number(n) => Some(n.to_string()),
                    _ => None,
                }).collect::<Vec<_>>().join(", ")),
                _ => None,
            }
        };
        match v {
            serde_yaml::Value::String(kind) => Ok(Cond { kind, params: Default::default() }),
            serde_yaml::Value::Mapping(m) if m.len() == 1 => {
                let (k, ps) = m.into_iter().next().unwrap();
                let kind = k.as_str().ok_or_else(|| D::Error::custom("a condition is named by a word"))?.to_string();
                let mut params = std::collections::BTreeMap::new();
                match ps {
                    serde_yaml::Value::Null => {}
                    serde_yaml::Value::Mapping(pm) => {
                        for (pk, pv) in pm {
                            let pk = pk.as_str().ok_or_else(|| D::Error::custom(format!("{kind}: parameter names are words")))?.to_string();
                            let pv = scalar(&pv).ok_or_else(|| D::Error::custom(format!("{kind}.{pk}: a word, a number or a list")))?;
                            params.insert(pk, pv);
                        }
                    }
                    _ => return Err(D::Error::custom(format!("{kind}: give its parameters as a map"))),
                }
                Ok(Cond { kind, params })
            }
            _ => Err(D::Error::custom("a condition is a name (mfa) or one name with its parameters (hours: {…})")),
        }
    }
}

impl Serialize for Cond {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        if self.params.is_empty() {
            return s.serialize_str(&self.kind);
        }
        let mut m = s.serialize_map(Some(1))?;
        m.serialize_entry(&self.kind, &self.params)?;
        m.end()
    }
}

/// A named network (`networks/<name>.yaml`): address ranges a rule can name
/// (`network: {name: office}`).
///
/// ```yaml
/// kind: a3t/network
/// name: office
/// ranges: [10.20.0.0/16, 192.0.2.10]
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkFile {
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub ranges: Vec<String>,
}

/// An address or a CIDR range (`10.20.0.0/16`, `192.0.2.10`, `2001:db8::/32`).
pub fn valid_range(r: &str) -> bool {
    match r.split_once('/') {
        Some((a, b)) => a.parse::<std::net::IpAddr>().is_ok_and(|ip| b.parse::<u8>().is_ok_and(|n| n <= if ip.is_ipv4() { 32 } else { 128 })),
        None => r.parse::<std::net::IpAddr>().is_ok(),
    }
}

impl NetworkFile {
    pub fn parse(yaml: &str) -> Result<Self, String> {
        let f: Self = serde_yaml::from_str(yaml).map_err(|e| format!("not a network file: {e}"))?;
        if f.kind != "a3t/network" {
            return Err(format!("kind is {:?}, not a3t/network", f.kind));
        }
        if f.name.is_empty() || !f.name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(format!("name {:?}: lowercase letters, digits and '-'", f.name));
        }
        if f.ranges.is_empty() {
            return Err("a network has at least one address range".into());
        }
        if let Some(bad) = f.ranges.iter().find(|r| !valid_range(r.trim())) {
            return Err(format!("{bad:?}: an IP address or a CIDR range (10.20.0.0/16)"));
        }
        Ok(f)
    }
}

/// `30m`, `4h`, `2d`, `90s` (or bare seconds) → seconds; between a minute and
/// thirty days.
pub fn approval_timeout_secs(t: &str) -> Result<u64, String> {
    let t = t.trim();
    let cut = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
    let (num, unit) = t.split_at(cut);
    let n: u64 = num.parse().map_err(|_| format!("timeout {t:?}: say e.g. 30m, 4h or 2d"))?;
    let secs = match unit.trim() {
        "" | "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        _ => return Err(format!("timeout {t:?}: say e.g. 30m, 4h or 2d")),
    };
    if !(60..=30 * 86_400).contains(&secs) {
        return Err(format!("timeout {t:?}: between 1m and 30d"));
    }
    Ok(secs)
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Who {
    #[serde(default)]
    pub roles: Vec<String>,
    /// `any`, or agent ids.
    #[serde(default)]
    pub agents: Option<AgentsSel>,
    /// People only: agents acting for them are not bound.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub people_only: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AgentsSel {
    Any(String),
    Named(Vec<String>),
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct On {
    /// Tool-name globs the rule is limited to (`github.*`).
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub models: Option<InOrNot>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InOrNot {
    #[serde(default, rename = "in")]
    pub is_in: Vec<String>,
    #[serde(default)]
    pub not_in: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoleFile {
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub extends: String,
    #[serde(default)]
    pub remove: Vec<String>,
}

/// Which kind of file a path in an org's policy repo holds (below the folder
/// A3T reads): `policies/`, `roles/` or `access/`, a `.yaml` or `.yml` file
/// directly in that folder. Anything else in the repo is not A3T's.
pub fn kind_of_path(path: &str) -> Option<&'static str> {
    let (dir, file) = path.split_once('/')?;
    if file.contains('/') || !(file.ends_with(".yaml") || file.ends_with(".yml")) {
        return None;
    }
    match dir {
        "policies" => Some("policy"),
        "roles" => Some("role"),
        // GOV-5: the org's own sign-in mapping (group → org role).
        "access" if file == "org.yaml" || file == "org.yml" => Some("org-access"),
        // RT-1: where the org's flows and agents may call out to.
        "egress" => Some("egress"),
        // Named address ranges that rules can name.
        "networks" => Some("network"),
        "access" => Some("access"),
        _ => None,
    }
}

/// Who holds which role in the org itself (`access/org.yaml`, GOV-5): the
/// org's sign-in groups mapped to its roles (`admin`, `securityadmin`,
/// `member`), and the role everyone else who signs in through its identity
/// provider gets (`default`; leave it out and they get none).
///
/// ```yaml
/// kind: a3t/org-access
/// groups: { acme-it: admin, acme-security: securityadmin, acme-staff: member }
/// default: member
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrgAccessFile {
    pub kind: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub groups: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub default: Option<String>,
}

pub const ORG_ROLES: &[&str] = &["admin", "securityadmin", "member"];

impl OrgAccessFile {
    pub fn parse(yaml: &str) -> Result<Self, String> {
        let f: Self = serde_yaml::from_str(yaml).map_err(|e| format!("not an org access file: {e}"))?;
        if f.kind != "a3t/org-access" {
            return Err(format!("kind is {:?}, not a3t/org-access", f.kind));
        }
        for (g, r) in &f.groups {
            if !ORG_ROLES.contains(&r.as_str()) {
                return Err(format!("group {g:?} → {r:?}: an org role is {}", ORG_ROLES.join(", ")));
            }
        }
        if let Some(d) = &f.default {
            if !ORG_ROLES.contains(&d.as_str()) {
                return Err(format!("default {d:?}: an org role is {}", ORG_ROLES.join(", ")));
            }
        }
        Ok(f)
    }

    /// The role a user's groups give them (the highest mapped), else `default`.
    pub fn role_for(&self, groups: &[String]) -> Option<String> {
        let rank = |r: &str| match r { "admin" => 3, "securityadmin" => 2, "member" => 1, _ => 0 };
        self.groups.iter()
            .filter(|(g, _)| groups.iter().any(|x| x == *g || x.trim_start_matches('/') == g.as_str()))
            .map(|(_, r)| r.as_str())
            .max_by_key(|r| rank(r))
            .or(self.default.as_deref())
            .map(str::to_string)
    }
}

/// Who holds which role on one of the org's projects (`access/<project>.yaml`).
/// People join IdP groups in the org's IdP; this file maps a group to a role.
/// An org with no IdP names A3T accounts instead (`users`).
///
/// ```yaml
/// kind: a3t/access
/// project: acme-support
/// groups: { support-team: Operator, support-leads: support-lead }
/// users: { jane@acme.com: Viewer }
/// ```
///
/// A role is a baseline role (Admin, Operator, Viewer) or one of the org's own
/// (`roles/<name>.yaml`, named without the org).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AccessFile {
    pub kind: String,
    pub project: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub groups: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub users: std::collections::BTreeMap<String, String>,
}

impl AccessFile {
    pub fn parse(yaml: &str) -> Result<Self, String> {
        let f: AccessFile = serde_yaml::from_str(yaml).map_err(|e| format!("access file: {e}"))?;
        if f.kind != "a3t/access" {
            return Err(format!("kind is {:?}, not a3t/access", f.kind));
        }
        if f.groups.is_empty() && f.users.is_empty() {
            return Err("an access file maps at least one group or user to a role".into());
        }
        Ok(f)
    }
}

impl PolicyFile {
    pub fn parse(yaml: &str) -> Result<Self, String> {
        serde_yaml::from_str(yaml).map_err(|e| format!("policy file: {e}"))
    }
}

/// One rule in plain words, for people who don't read Cedar.
pub fn describe_rule(r: &Rule) -> String {
    let cat = crate::catalogue::catalogue();
    let label = |id: &str| -> String {
        if let Some(p) = cat.permissions.iter().find(|p| p.id == id) {
            return p.label.to_lowercase();
        }
        match id.strip_prefix("tools.") {
            Some(glob) => format!("call tools named {glob}"),
            None => id.to_string(),
        }
    };
    let who = match &r.who {
        None => "Nobody".to_string(),
        Some(w) => {
            let mut parts: Vec<String> = Vec::new();
            if !w.roles.is_empty() {
                parts.push(format!("{} users", w.roles.join(" or ")));
            }
            match &w.agents {
                Some(AgentsSel::Any(_)) => parts.push("agents".into()),
                Some(AgentsSel::Named(ids)) if !ids.is_empty() => parts.push(format!("agents {}", ids.join(", "))),
                _ => {}
            }
            if parts.is_empty() { "Nobody".into() } else { parts.join(" and ") }
        }
    };
    let approval = !r.needs_approval.is_empty();
    let who = if approval && who == "Nobody" { "Agents".to_string() } else { who };
    let verb = if approval { "need a person's approval to" } else if who == "Nobody" { "may" } else { "may not" };
    let items = if approval { &r.needs_approval } else { &r.cannot };
    let what = items.iter().map(|c| label(c)).collect::<Vec<_>>().join(", ");
    let mut on = Vec::new();
    if let Some(o) = &r.on {
        if !o.tools.is_empty() {
            on.push(format!("tools named {}", o.tools.join(" or ")));
        }
        if let Some(m) = &o.models {
            if !m.is_in.is_empty() { on.push(format!("the models {}", m.is_in.join(", "))); }
            if !m.not_in.is_empty() { on.push(format!("models other than {}", m.not_in.join(", "))); }
        }
    }
    if approval {
        if let Some(t) = &r.timeout { on.push(format!("asked for up to {t}")); }
    }
    let on = if on.is_empty() { String::new() } else { format!(" ({})", on.join("; ")) };
    format!("{who} {verb} {what}{on}. {}", r.reason.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_read_as_sentences() {
        let f = PolicyFile::parse("kind: a3t/policy\nname: x\nrules:\n  - who: { roles: [Operator], agents: any }\n    cannot: [models.use, tools.delete_*]\n    on: { models: { not_in: [acme/sonnet] } }\n    reason: Approved models only.\n").unwrap();
        assert_eq!(describe_rule(&f.rules[0]),
            "Operator users and agents may not call language models, call tools named delete_* (models other than acme/sonnet). Approved models only.");
        let g = PolicyFile::parse("kind: a3t/policy\nname: x\nrules:\n  - cannot: [flows.delete]\n    reason: Never.\n").unwrap();
        assert_eq!(describe_rule(&g.rules[0]), "Nobody may delete flows. Never.");
        let o = OrgAccessFile::parse("kind: a3t/org-access\ngroups: { it: admin, sec: securityadmin }\ndefault: member\n").unwrap();
        assert_eq!(o.role_for(&["/it".into(), "sec".into()]), Some("admin".into()));
        assert_eq!(o.role_for(&["sec".into()]), Some("securityadmin".into()));
        assert_eq!(o.role_for(&[]), Some("member".into()));
        assert!(OrgAccessFile::parse("kind: a3t/org-access\ngroups: { it: owner }\n").is_err());
        assert_eq!(kind_of_path("access/org.yaml"), Some("org-access"));
        let a = PolicyFile::parse("kind: a3t/policy\nname: x\nrules:\n  - needs_approval: [tools.delete_*]\n    timeout: 4h\n    reason: A person checks deletes.\n").unwrap();
        assert_eq!(describe_rule(&a.rules[0]), "Agents need a person's approval to call tools named delete_* (asked for up to 4h). A person checks deletes.");
        assert_eq!(approval_timeout_secs("4h"), Ok(14_400));
        assert!(approval_timeout_secs("10s").is_err());
        assert!(approval_timeout_secs("4w").is_err());
    }
}
