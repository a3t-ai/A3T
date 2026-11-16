//! The A3T permission catalogue: every permission a role or rule can name.
//!
//! The catalogue is config-to-code, owned by the A3T repo
//! (`a3t/policies/catalogue.yaml`) and compiled into each release. A permission
//! is an object and a verb (`flows.run`) and lists what enforces it: MCP tools
//! (checked as `mcp:call` / `tool:invoke` on `Tool::"<name>"`), Cedar actions,
//! or the console (app visibility, which is never the guard on its own).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// The catalogue file as shipped with this release.
pub const CATALOGUE_YAML: &str = include_str!("../../../a3t/policies/catalogue.yaml");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalogue {
    pub kind: String,
    pub version: u32,
    /// Verb → its plain label ("run" → "Run it").
    pub verbs: BTreeMap<String, String>,
    pub permissions: Vec<Permission>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permission {
    /// `<object>.<verb>`, or `apps.<app>` / `tools.external`.
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    /// `console` for app visibility; absent for server-enforced permissions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforced_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Catalogue {
    pub fn parse(yaml: &str) -> Result<Self, String> {
        let c: Catalogue = serde_yaml::from_str(yaml).map_err(|e| format!("catalogue: {e}"))?;
        c.check()?;
        Ok(c)
    }

    /// The structural rules the file must keep (tool coverage is checked where
    /// the tool list lives, in flowr-mcp-core).
    pub fn check(&self) -> Result<(), String> {
        if self.kind != "a3t/catalogue" {
            return Err(format!("catalogue: kind is {:?}, not a3t/catalogue", self.kind));
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut tool_owner: BTreeMap<&str, &str> = BTreeMap::new();
        for p in &self.permissions {
            if !ids.insert(p.id.as_str()) {
                return Err(format!("catalogue: permission {} is listed twice", p.id));
            }
            let Some((object, verb)) = p.id.split_once('.') else {
                return Err(format!("catalogue: {} is not <object>.<verb>", p.id));
            };
            let console = p.enforced_by.as_deref() == Some("console");
            if object == "apps" {
                if !console || !p.tools.is_empty() || !p.actions.is_empty() {
                    return Err(format!("catalogue: {} is an app: enforced_by console, no tools or actions", p.id));
                }
                continue;
            }
            if !(object == "tools" && verb == "external") && !self.verbs.contains_key(verb) {
                return Err(format!("catalogue: {} uses an unknown verb {verb:?}", p.id));
            }
            if p.tools.is_empty() && p.actions.is_empty() {
                return Err(format!("catalogue: {} is enforced by nothing", p.id));
            }
            for a in &p.actions {
                if !crate::actions::ALL.contains(&a.as_str()) {
                    return Err(format!("catalogue: {} names {a:?}, which FlowR never checks", p.id));
                }
            }
            for t in &p.tools {
                if let Some(other) = tool_owner.insert(t.as_str(), p.id.as_str()) {
                    return Err(format!("catalogue: tool {t} is in both {other} and {}", p.id));
                }
            }
        }
        Ok(())
    }

    /// The permission a built-in MCP tool belongs to.
    pub fn permission_of_tool(&self, tool: &str) -> Option<&Permission> {
        self.permissions.iter().find(|p| p.tools.iter().any(|t| t == tool))
    }
}

/// The catalogue shipped with this release (parsed once).
pub fn catalogue() -> &'static Catalogue {
    static C: OnceLock<Catalogue> = OnceLock::new();
    C.get_or_init(|| Catalogue::parse(CATALOGUE_YAML).expect("the shipped catalogue is checked by its tests"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_catalogue_is_valid() {
        Catalogue::parse(CATALOGUE_YAML).unwrap();
    }

    #[test]
    fn every_checked_action_has_a_permission() {
        let c = catalogue();
        for a in crate::actions::ALL {
            assert!(c.permissions.iter().any(|p| p.actions.iter().any(|x| x == a)), "no permission names {a}");
        }
    }

    #[test]
    fn mistakes_are_refused() {
        let base = "kind: a3t/catalogue\nversion: 1\nverbs: {read: See it}\npermissions:\n";
        assert!(Catalogue::parse(&format!("{base}  - {{id: flows.read, label: x}}\n")).is_err(), "enforced by nothing");
        assert!(Catalogue::parse(&format!("{base}  - {{id: flows.fly, label: x, tools: [a]}}\n")).is_err(), "unknown verb");
        assert!(Catalogue::parse(&format!("{base}  - {{id: flows.read, label: x, actions: [\"nope:x\"]}}\n")).is_err(), "unknown action");
        assert!(Catalogue::parse(&format!("{base}  - {{id: a.read, label: x, tools: [t]}}\n  - {{id: b.read, label: y, tools: [t]}}\n")).is_err(), "tool twice");
    }
}
