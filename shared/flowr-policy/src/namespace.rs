//! Namespace access — who may see and change what in which project.
//!
//! A namespace is a project (A3T's Projects app). Cedar decides *what kind* of
//! step a principal may take; this decides *where*: a member of org A must not
//! list, read, run or change org B's flows, resources, triggers, runs or
//! runtimes, whatever their org-wide role.
//!
//! The rule itself lives with the project data (FlowR CE `access.rs`, over the
//! tables the A3T project flows keep); this module is the seam every boundary
//! asks — MCP tools, the registry, ops and canvas routes. The default guard
//! allows everything, so a FlowR without projects (CE) behaves as before.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};

use async_trait::async_trait;

use crate::Principal;

/// What a principal may do in a namespace, weakest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NsAccess {
    /// Not even see that it exists.
    None,
    /// List and read.
    Read,
    /// Read, and run its flows.
    Run,
    /// Read, run, and create / change / delete.
    Write,
}

/// Kinds of items, for [`NamespaceGuard::effective_namespace`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Flow,
    Resource,
    Trigger,
    Agent,
}

#[async_trait]
pub trait NamespaceGuard: Send + Sync {
    /// What `principal` may do in `namespace`.
    async fn access(&self, principal: &Principal, namespace: &str) -> NsAccess;

    /// A namespace the platform itself owns (`a3t`, `system`): readable by
    /// everyone signed in, its own flows run by anyone, its runs private to who
    /// ran them.
    fn is_platform(&self, _namespace: &str) -> bool {
        false
    }

    /// The namespace an item really belongs to. Before projects, users' work was
    /// saved in the platform namespace; a guard maps that work to the project
    /// that owns it, so it is protected like the rest of that project.
    fn effective_namespace(&self, _kind: ItemKind, namespace: &str, _name: &str, _type_ref: &str, _created_by: &str) -> String {
        namespace.to_string()
    }

    /// Whether `principal` may control (restart, pause, terminate) a runtime with
    /// these labels. A shared runtime is the platform's: superadmin only. A
    /// runtime labelled `a3t.io/org` is that org's: its admins too.
    async fn can_control_runtime(&self, _principal: &Principal, _labels: &BTreeMap<String, String>) -> bool {
        true
    }

    /// Whether `principal` may see a runtime with these labels at all (the
    /// shared ones always; an org's only by its members).
    async fn can_see_runtime(&self, _principal: &Principal, _labels: &BTreeMap<String, String>) -> bool {
        true
    }

    /// Whether a flow of `namespace` may run on a runtime with these labels —
    /// the runtime policy: an org (or a project) set to *isolated* runs only on
    /// its own runtimes; nothing runs on another org's runtime.
    async fn runtime_allowed(&self, _namespace: &str, _labels: &BTreeMap<String, String>) -> bool {
        true
    }

    /// False for the default guard, which allows everything.
    fn enforcing(&self) -> bool {
        false
    }
}

/// Allows everything (CE, and before a guard is installed).
pub struct AllowAllNamespaces;

#[async_trait]
impl NamespaceGuard for AllowAllNamespaces {
    async fn access(&self, _principal: &Principal, _namespace: &str) -> NsAccess {
        NsAccess::Write
    }
}

fn slot() -> &'static RwLock<Arc<dyn NamespaceGuard>> {
    static SLOT: OnceLock<RwLock<Arc<dyn NamespaceGuard>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(Arc::new(AllowAllNamespaces)))
}

/// The installed guard.
pub fn namespace_guard() -> Arc<dyn NamespaceGuard> {
    slot().read().unwrap().clone()
}

/// Install a guard (FlowR EE, when policy is enforcing).
pub fn set_namespace_guard(guard: Arc<dyn NamespaceGuard>) {
    *slot().write().unwrap() = guard;
}

/// The human a principal acts for: a user, or the user an agent acts on behalf
/// of. `None` for a service (internal machinery, trusted) or an unattended agent.
pub fn acting_user(principal: &Principal) -> Option<&str> {
    match principal {
        Principal::Human { user_id, .. } => Some(user_id),
        Principal::Agent { .. } => principal.on_behalf_of(),
        Principal::Service { .. } => None,
    }
}

/// Whether a principal holds the SuperAdmin role.
pub fn is_superadmin(principal: &Principal) -> bool {
    principal.roles().iter().any(|r| r == crate::roles::SUPER_ADMIN)
}

// ── Roles in the project a call touches (org SSO spec R6/R7) ──
//
// A user's tenant role is per org and per project, not global: the same person
// can be admin in their own org and viewer in another. Cedar's baseline speaks
// in roles (`Admin` may author, `Viewer` may only read), so before a decision
// the principal's roles are replaced by what they hold IN THE PROJECT THE CALL
// TOUCHES: write → Admin (the developer tier), run → Operator, read → Viewer.
// Platform roles (SuperAdmin, SecurityAdmin) are kept. A global Keycloak
// `viewer`/`operator`/`admin` thus neither caps nor widens access inside an
// org's project; with no project in the call the global roles apply as before.

/// The project (namespace) a policy request touches, if it names one: a
/// `namespace` attribute or argument, a flow definition's `document.namespace`,
/// or a `<namespace>/<name>` flow id.
pub fn request_namespace(req: &crate::ExecutionRequest) -> Option<String> {
    let s = |v: Option<&serde_json::Value>| v.and_then(|x| x.as_str()).map(str::trim).filter(|x| !x.is_empty()).map(String::from);
    if let Some(ns) = s(req.resource.attributes.get("namespace")) {
        return Some(ns);
    }
    let p = &req.params;
    if let Some(ns) = s(p.get("namespace")) {
        return Some(ns);
    }
    if let Some(ns) = s(p.get("json").and_then(|j| j.get("document")).and_then(|d| d.get("namespace"))) {
        return Some(ns);
    }
    if let Some(y) = p.get("yaml").and_then(|y| y.as_str()) {
        if let Ok(doc) = serde_yaml::from_str::<serde_json::Value>(y) {
            if let Some(ns) = s(doc.get("document").and_then(|d| d.get("namespace"))) {
                return Some(ns);
            }
        }
    }
    if req.resource.type_name == "Flow" {
        if let Some((ns, _)) = req.resource.id.split_once('/') {
            if !ns.is_empty() {
                return Some(ns.to_string());
            }
        }
    }
    None
}

/// The tenant roles that an access level carries.
pub fn tier_roles(access: NsAccess) -> &'static [&'static str] {
    use crate::roles::{ADMIN, OPERATOR, VIEWER};
    match access {
        NsAccess::Write => &[ADMIN, OPERATOR, VIEWER],
        NsAccess::Run => &[OPERATOR, VIEWER],
        NsAccess::Read => &[VIEWER],
        NsAccess::None => &[],
    }
}

/// Replace a principal's tenant roles by those it holds in `access`; platform
/// roles stay.
pub fn with_tier(principal: &Principal, access: NsAccess) -> Principal {
    use crate::roles::{SECURITY_ADMIN, SUPER_ADMIN};
    let scope = |roles: &[String]| -> Vec<String> {
        let mut out: Vec<String> = roles.iter().filter(|r| *r == SUPER_ADMIN || *r == SECURITY_ADMIN).cloned().collect();
        for r in tier_roles(access) {
            if !out.iter().any(|x| x == r) {
                out.push((*r).to_string());
            }
        }
        out
    };
    let mut p = principal.clone();
    match &mut p {
        Principal::Human { roles, .. } | Principal::Agent { roles, .. } => *roles = scope(roles),
        Principal::Service { .. } => {}
    }
    p
}

/// The principal as policy should see it for this request: scoped to the
/// project the call touches when the installed guard enforces projects and the
/// call names one; unchanged otherwise (no project, a service, CE).
pub async fn scoped_principal(req: &crate::ExecutionRequest) -> Principal {
    let guard = namespace_guard();
    if !guard.enforcing() || acting_user(&req.principal).is_none() {
        return req.principal.clone();
    }
    let Some(ns) = request_namespace(req) else {
        return req.principal.clone();
    };
    let access = guard.access(&req.principal, &ns).await;
    with_tier(&req.principal, access)
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    use serde_json::json;

    fn req(params: serde_json::Value) -> crate::ExecutionRequest {
        crate::ExecutionRequest {
            principal: Principal::Human { user_id: "u".into(), roles: vec!["Viewer".into()], memory_branch: None },
            action: crate::actions::mcp_call(),
            resource: crate::Resource::tool("create_flow"),
            context: Default::default(),
            attributes: Default::default(),
            params,
        }
    }

    #[test]
    fn finds_the_namespace_a_call_touches() {
        assert_eq!(request_namespace(&req(json!({"namespace": "acme-p1"}))).as_deref(), Some("acme-p1"));
        let y = "document:\n  dsl: 1.0.0\n  namespace: acme-p1\n  name: f\n  version: 0.0.1\ndo: []\n";
        assert_eq!(request_namespace(&req(json!({"yaml": y}))).as_deref(), Some("acme-p1"));
        assert_eq!(request_namespace(&req(json!({"json": {"document": {"namespace": "b"}}}))).as_deref(), Some("b"));
        assert_eq!(request_namespace(&req(json!({}))), None);
    }

    #[test]
    fn the_project_role_replaces_the_global_tenant_role() {
        let viewer = Principal::Human { user_id: "u".into(), roles: vec!["Viewer".into()], memory_branch: None };
        assert_eq!(with_tier(&viewer, NsAccess::Write).roles(), ["Admin", "Operator", "Viewer"]);
        let dev = Principal::Human { user_id: "u".into(), roles: vec!["Admin".into(), "Operator".into(), "Viewer".into()], memory_branch: None };
        assert_eq!(with_tier(&dev, NsAccess::Read).roles(), ["Viewer"]);
        let sec = Principal::Human { user_id: "u".into(), roles: vec!["SecurityAdmin".into()], memory_branch: None };
        assert_eq!(with_tier(&sec, NsAccess::None).roles(), ["SecurityAdmin"]);
    }
}
