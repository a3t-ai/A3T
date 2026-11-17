//! The policy gate: what is saved as a Cedar policy always compiles, and takes
//! effect at once, everywhere.
//!
//! Writing a `type_ref: cedar` resource goes through the registry store, by
//! whichever door (REST, MCP, share/clone). Before it is written, the gate checks
//! that the whole policy set still loads with the change; a policy that does not
//! compile is refused with Cedar's error and the live set stays as it was. After
//! it is written (or deleted), the gate reloads the set on this FlowR and tells
//! the others. The Cedar engine is EE: FlowR EE installs the gate; with none
//! installed (CE, AllowAll) policies are stored as given.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

#[async_trait]
pub trait PolicyGate: Send + Sync {
    /// Would the saved policies still load with `namespace/name` set to `source`
    /// (`None`: removed)? The error says which policy and what is wrong.
    async fn check(&self, namespace: &str, name: &str, source: Option<&str>) -> Result<(), String>;
    /// A policy changed: load the set here, and on every other FlowR.
    async fn changed(&self);
    /// Compile an agent's own policy (`kind: a3t/policy`, applying to agent
    /// `agent_id` of `project`): `{name, placement, hidden_apps}` or why not.
    async fn compile_agent_policy(&self, project: &str, agent_id: &str, yaml: &str) -> Result<serde_json::Value, String> {
        let _ = (project, agent_id, yaml);
        Err("agent security policies need FlowR EE".into())
    }
    /// Compile an org's policy repo files (`(path, text)`) for its `projects`,
    /// all or nothing, and check they load with every other saved policy:
    /// `{files, hidden_apps, access}`, or one error per bad file.
    async fn compile_org_files(&self, org: &str, projects: &[String], files: &[(String, String)]) -> Result<serde_json::Value, Vec<String>> {
        let _ = (org, projects, files);
        Err(vec!["org policy files need FlowR EE".into()])
    }
}

static GATE: OnceLock<Arc<dyn PolicyGate>> = OnceLock::new();

pub fn install(gate: Arc<dyn PolicyGate>) {
    let _ = GATE.set(gate);
}

pub fn get() -> Option<Arc<dyn PolicyGate>> {
    GATE.get().cloned()
}

/// The Cedar source a policy resource's config carries.
pub fn source_of(config: &serde_json::Value) -> Option<&str> {
    config.get("cedar").and_then(|v| v.as_str())
}
