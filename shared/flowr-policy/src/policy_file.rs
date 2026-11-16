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
    pub cannot: Vec<String>,
    #[serde(default)]
    pub on: Option<On>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Who {
    #[serde(default)]
    pub roles: Vec<String>,
    /// `any`, or agent ids.
    #[serde(default)]
    pub agents: Option<AgentsSel>,
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
    let verb = if who == "Nobody" { "may" } else { "may not" };
    let what = r.cannot.iter().map(|c| label(c)).collect::<Vec<_>>().join(", ");
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
    }
}
