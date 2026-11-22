//! Policy files that no longer compile (GOV-4).
//!
//! Every file is checked when it is saved, so what is stored compiled then.
//! A file can still stop compiling later — the permission catalogue or the
//! schema changed in a release. Such a file is never silently left out: what
//! it guarded is closed (its org's projects, or its agent, refuse calls with a
//! reason naming the file) until a security admin fixes it, the break is
//! written to the decision log once (`policy:broken`) and when it heals
//! (`policy:healed`), and `GET /security/policy-health` lists it for the
//! console (Security and Studio mark the file).

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Broken {
    /// `org` or `agent`.
    pub kind: String,
    /// The org's id, or the agent's id.
    pub owner: String,
    /// The org's id the file belongs to (for an agent, its org when known).
    pub org: Option<String>,
    /// What is wrong, one line per file.
    pub errors: Vec<String>,
    /// What is closed until it is fixed: projects, or the agent.
    pub closed: Vec<String>,
    /// When it was first seen broken (RFC 3339).
    pub since: String,
}

fn state() -> &'static Mutex<BTreeMap<String, Broken>> {
    static S: OnceLock<Mutex<BTreeMap<String, Broken>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// Replace what is known to be broken with this load's findings. Returns
/// (newly broken, healed), so each change is logged once per pod.
pub fn set(found: Vec<Broken>) -> (Vec<Broken>, Vec<Broken>) {
    let mut s = state().lock().expect("policy health lock");
    let mut next = BTreeMap::new();
    let mut fresh = Vec::new();
    for mut b in found {
        let key = format!("{}:{}", b.kind, b.owner);
        match s.get(&key) {
            Some(old) if old.errors == b.errors => b.since = old.since.clone(),
            _ => fresh.push(b.clone()),
        }
        next.insert(key, b);
    }
    let healed = s.iter().filter(|(k, _)| !next.contains_key(*k)).map(|(_, v)| v.clone()).collect();
    *s = next;
    (fresh, healed)
}

pub fn list() -> Vec<Broken> {
    state().lock().expect("policy health lock").values().cloned().collect()
}

/// A Cedar statement that refuses every call in its project, naming why. The
/// org's narrowing rules can't be enforced, so nothing runs there unguarded.
pub fn closing_statement(id: &str, reason: &str) -> String {
    let q = |s: &str| serde_json::to_string(s).expect("a string serializes");
    format!("@id({})\n@reason({})\nforbid(principal, action, resource);\n", q(id), q(reason))
}

/// The same, for one agent wherever it runs.
pub fn closing_agent_statement(id: &str, agent: &str, reason: &str) -> String {
    let q = |s: &str| serde_json::to_string(s).expect("a string serializes");
    format!(
        "@id({})\n@reason({})\nforbid(principal, action, resource) when {{\n  (principal has template && principal.template has agent && principal.template.agent == {})\n}};\n",
        q(id), q(reason), q(agent)
    )
}

/// Log a change in health to the decision log, once.
pub fn log(fresh: &[Broken], healed: &[Broken]) {
    let platform = flowr_policy::Principal::Service { memory_branch: None, service_id: "policy-loader".into() };
    for b in fresh {
        tracing::error!(kind = %b.kind, owner = %b.owner, errors = ?b.errors, closed = ?b.closed, "policy files no longer compile; what they guard is closed until they are fixed");
        let note = format!("{} closed: {}", b.closed.join(", "), b.errors.join(" | "));
        flowr_policy::audit::event(&platform, "policy:broken", "PolicyFile", &format!("{}:{}", b.kind, b.owner),
            None, b.org.as_deref(), Some("policy files do not compile"), Some(&note));
    }
    for b in healed {
        tracing::info!(kind = %b.kind, owner = %b.owner, "policy files compile again; reopened");
        flowr_policy::audit::event(&platform, "policy:healed", "PolicyFile", &format!("{}:{}", b.kind, b.owner),
            None, b.org.as_deref(), None, Some(&format!("{} reopened", b.closed.join(", "))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(owner: &str, e: &str) -> Broken {
        Broken { kind: "org".into(), owner: owner.into(), org: Some(owner.into()), errors: vec![e.into()], closed: vec![], since: "t0".into() }
    }

    #[test]
    fn each_change_is_reported_once() {
        let (f, h) = set(vec![b("acme", "x")]);
        assert_eq!((f.len(), h.len()), (1, 0));
        let mut again = b("acme", "x");
        again.since = "t1".into();
        let (f, h) = set(vec![again]);
        assert_eq!((f.len(), h.len()), (0, 0));
        assert_eq!(list()[0].since, "t0", "kept from when it broke");
        let (f, h) = set(vec![]);
        assert_eq!((f.len(), h.len()), (0, 1));
    }
}
