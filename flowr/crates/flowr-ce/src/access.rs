//! Who may do what in which namespace — the project rules, enforced.
//!
//! The A3T project flows (a3t/backend/flows/projects) own the data: `projects`,
//! `org_members` and `orgs` in the platform DB. This guard reads them (a snapshot
//! refreshed every few seconds) and answers the same question the `a3t-projects`
//! flow answers for listing — what the caller may do in a namespace — so the
//! server refuses what the dash hides. FlowR EE installs it when policy is
//! enforcing (`install`); CE keeps allow-all.
//!
//! - A personal project: its owner; members by role; superadmin reads it unless
//!   it is private.
//! - An org project: members by role; the org's admins write, its security
//!   admins read; every org member reads it unless it is private; superadmin.
//! - A namespace from before projects (`default`, …): org `default`'s.
//! - The platform (`a3t`, `system`): everyone reads and runs it; superadmin and
//!   the owning org's admins (orgs `a3t` / `system`) write.
//! - Runtimes: an `a3t.io/org`-labelled runtime is that org's (its admins
//!   control it); every other runtime is shared and only superadmin controls it.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;

use flowr_policy::namespace::{acting_user, is_superadmin, ItemKind, NamespaceGuard, NsAccess};
use flowr_policy::Principal;
use flowr_storage::StorageBackend;

use crate::handlers::call_lancedb::{batches_to_json, DEFAULT_PLATFORM_DB_PATH, PLATFORM_DB_RESOURCE};

const PLATFORM: &[&str] = &["a3t", "system"];
/// The org namespaces from before projects belong to: org `a3t` (its id stays
/// `org-default`, which users, agents and settings carry).
const LEGACY_ORG: &str = "org-default";
/// The platform orgs and the namespaces they own: `a3t` runs the UI's flows,
/// `system` installs A3T and keeps the SaaS platform running.
const PLATFORM_ORGS: &[(&str, &str)] = &[("a3t", "org-default"), ("system", "org-system")];
/// The label that makes a runtime an org's own.
pub const ORG_LABEL: &str = "a3t.io/org";
/// The label that makes a runtime a single project's own (isolated personal projects).
pub const PROJECT_LABEL: &str = "a3t.io/project";
const REFRESH: Duration = Duration::from_secs(5);
static PROJECT_GUARD: std::sync::OnceLock<Arc<ProjectGuard>> = std::sync::OnceLock::new();

#[derive(Debug, Clone, Default)]
struct Project {
    owner_kind: String,
    /// The owner's handle (user or org).
    owner: String,
    owner_id: String,
    private: bool,
    members: Vec<(String, String)>,
    /// `isolated` | `shared` | empty (the org's setting).
    runtime: String,
    /// Who else may use it (read, clone, run — never change): `public` = every
    /// org; `orgs` = the org handles in `exposed_to`; empty = nobody.
    exposure: String,
    exposed_to: Vec<String>,
}

/// The rows the decision needs, as of `loaded`.
#[derive(Debug, Default)]
pub struct Snapshot {
    projects: HashMap<String, Project>,
    /// (org id, user id) → member | admin | securityadmin
    org_roles: HashMap<(String, String), String>,
    /// org handle → org id
    orgs: HashMap<String, String>,
    /// org handle → runtime policy (`isolated` | `shared`)
    org_runtime: HashMap<String, String>,
    /// org handle → the most runs active at once (`settings.quota.max_active_runs`, AC-18)
    org_max_runs: HashMap<String, usize>,
    /// user id → (email, name) as project members and org members record them
    /// (people who sign in through an org's own IdP may have no realm account).
    contacts: HashMap<String, (String, String)>,
}

impl Snapshot {
    fn from_rows(projects: &[Value], members: &[Value], orgs: &[Value]) -> Self {
        let s = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let mut snap = Snapshot::default();
        for p in projects {
            let listed = serde_json::from_str::<Vec<Value>>(&s(p, "members")).unwrap_or_default();
            for m in &listed {
                snap.note_contact(&s(m, "id"), &s(m, "email"), &s(m, "name"));
            }
            let members = listed.iter().map(|m| (s(m, "id"), s(m, "role"))).filter(|(id, _)| !id.is_empty()).collect();
            let policy = serde_json::from_str::<Value>(&s(p, "policy")).unwrap_or(Value::Null);
            let runtime = policy.get("runtime").and_then(|r| r.as_str()).unwrap_or_default().to_string();
            let exposure = policy.get("exposure").and_then(|e| e.get("audience")).and_then(|a| a.as_str()).unwrap_or_default().to_string();
            let exposed_to = policy.get("exposure").and_then(|e| e.get("orgs")).and_then(|o| o.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            snap.projects.insert(s(p, "namespace"), Project {
                owner_kind: s(p, "owner_kind"), owner: s(p, "owner"), owner_id: s(p, "owner_id"),
                private: s(p, "private") == "true", members, runtime, exposure, exposed_to,
            });
        }
        for m in members {
            snap.note_contact(&s(m, "user_id"), &s(m, "email"), &s(m, "name"));
            snap.org_roles.insert((s(m, "org_id"), s(m, "user_id")), s(m, "role"));
        }
        for o in orgs {
            snap.orgs.entry(s(o, "slug")).or_insert_with(|| s(o, "id"));
            let runtime = serde_json::from_str::<Value>(&s(o, "settings")).ok()
                .and_then(|v| v.get("runtime").and_then(|r| r.as_str()).map(String::from)).unwrap_or_default();
            if !runtime.is_empty() {
                snap.org_runtime.insert(s(o, "slug"), runtime);
            }
            let max_runs = serde_json::from_str::<Value>(&s(o, "settings")).ok()
                .and_then(|v| v.pointer("/quota/max_active_runs").and_then(|n| n.as_u64()));
            if let Some(m) = max_runs {
                snap.org_max_runs.insert(s(o, "slug"), m as usize);
            }
        }
        snap
    }

    fn note_contact(&mut self, user: &str, email: &str, name: &str) {
        if user.is_empty() || (email.is_empty() && name.is_empty()) {
            return;
        }
        let e = self.contacts.entry(user.to_string()).or_default();
        if e.0.is_empty() { e.0 = email.to_string(); }
        if e.1.is_empty() { e.1 = name.to_string(); }
    }

    /// `(email, name)` as the project and org rows record `user`.
    pub fn contact(&self, user: &str) -> Option<(String, String)> {
        self.contacts.get(user).cloned()
    }

    fn org_role(&self, org_id: &str, user: &str) -> Option<&str> {
        self.org_roles.get(&(org_id.to_string(), user.to_string())).map(String::as_str)
    }

    /// Whether a flow of `namespace` may run on a runtime with `labels`.
    /// - Nothing runs on another org's runtime (`a3t.io/org`) or another
    ///   project's (`a3t.io/project`).
    /// - Isolated — the project's `runtime` policy, else its org's — runs only on
    ///   its own: an org project on `a3t.io/org=<org>`, a personal project on
    ///   `a3t.io/project=<namespace>`.
    /// - Shared runs on the shared runtimes (no owner label) or its org's own.
    /// - The platform's flows run on shared runtimes only. A namespace from before
    ///   projects is org `default`'s.
    pub fn runtime_allowed(&self, namespace: &str, labels: &BTreeMap<String, String>) -> bool {
        let rt_org = labels.get(ORG_LABEL).map(String::as_str);
        let rt_project = labels.get(PROJECT_LABEL).map(String::as_str);
        if PLATFORM.contains(&namespace) {
            return rt_org.is_none() && rt_project.is_none();
        }
        let (org, policy) = match self.projects.get(namespace) {
            Some(p) if p.owner_kind == "org" => {
                let policy = if p.runtime.is_empty() { self.org_runtime.get(&p.owner).cloned().unwrap_or_default() } else { p.runtime.clone() };
                (Some(p.owner.as_str()), policy)
            }
            Some(p) => (None, p.runtime.clone()),
            None => (Some("default"), self.org_runtime.get("default").cloned().unwrap_or_default()),
        };
        if rt_project.is_some_and(|p| p != namespace) || rt_org.is_some_and(|o| Some(o) != org) {
            return false;
        }
        if policy == "isolated" {
            return match org {
                Some(o) => rt_org == Some(o),
                None => rt_project == Some(namespace),
            };
        }
        true
    }

    /// The decision, for a known user (`super_` = holds SuperAdmin).
    pub fn decide(&self, user: &str, super_: bool, namespace: &str) -> NsAccess {
        if PLATFORM.contains(&namespace) {
            let org = PLATFORM_ORGS.iter().find(|(ns, _)| *ns == namespace).map(|(_, id)| *id).unwrap_or_default();
            return if super_ || self.org_role(org, user) == Some("admin") { NsAccess::Write } else { NsAccess::Run };
        }
        // The project roles of the role table (ID-1).
        let by_role = |role: &str| match flowr_policy::roles::project_access(role) {
            Some("write") => NsAccess::Write,
            Some("run") => NsAccess::Run,
            Some("read") => NsAccess::Read,
            _ => NsAccess::None,
        };
        let Some(p) = self.projects.get(namespace) else {
            // (no project row: the pre-projects rule below)
            // From before projects: org default's.
            return match (super_, self.org_role(LEGACY_ORG, user)) {
                (true, _) | (_, Some("admin" | "member")) => NsAccess::Write,
                (_, Some("securityadmin")) => NsAccess::Read,
                _ => NsAccess::None,
            };
        };
        let member = p.members.iter().find(|(id, _)| id == user).map(|(_, r)| by_role(r));
        let own = if p.owner_kind == "user" {
            if p.owner_id == user {
                NsAccess::Write
            } else if p.private {
                member.unwrap_or(NsAccess::None)
            } else {
                member.unwrap_or(if super_ { NsAccess::Read } else { NsAccess::None })
            }
        } else {
            let org = match self.org_role(&p.owner_id, user) {
                Some("admin") => NsAccess::Write,
                Some("securityadmin") => NsAccess::Read,
                Some(_) if !p.private => NsAccess::Read,
                _ => NsAccess::None,
            };
            let sup = if super_ { NsAccess::Write } else { NsAccess::None };
            member.unwrap_or(NsAccess::None).max(org).max(sup)
        };
        own.max(self.exposed(p, user))
    }

    /// What an exposed project gives someone outside it: read, clone and run —
    /// never change. `public`: anyone signed in; `orgs`: members of those orgs.
    /// The `system` org exposes to org `a3t` only — never directly to everyone;
    /// a3t decides what it passes on.
    fn exposed(&self, p: &Project, user: &str) -> NsAccess {
        let system = p.owner_kind == "org" && p.owner == "system";
        let member_of = |h: &str| self.orgs.get(h).is_some_and(|id| self.org_role(id, user).is_some());
        match p.exposure.as_str() {
            "public" if !system => NsAccess::Run,
            "orgs" if p.exposed_to.iter().any(|h| (!system || h == "a3t") && member_of(h)) => NsAccess::Run,
            _ => NsAccess::None,
        }
    }

    /// Every way someone reaches `namespace`, from the same rows and rules as
    /// [`Snapshot::decide`]: the people it names (each route: owner, member
    /// role, org role, SuperAdmin) and the groups too wide to name (everyone
    /// signed in, another org's members through exposure). `supers` holds the
    /// SuperAdmins (their role lives in the IdP, not here). A person's best
    /// route is what `decide` gives them — the tests hold the two together.
    pub fn routes(&self, namespace: &str, supers: &[String]) -> ProjectRoutes {
        use NsAccess::{Read, Run, Write};
        let mut people: Vec<Route> = Vec::new();
        let push = |people: &mut Vec<Route>, user: &str, capacity: &'static str, access: NsAccess| {
            if !user.is_empty() && access > NsAccess::None {
                people.push(Route { user: user.to_string(), capacity, access: access_name(access) });
            }
        };
        let org_rows = |org_id: &str| -> Vec<(String, String)> {
            let mut v: Vec<(String, String)> = self.org_roles.iter()
                .filter(|((o, _), _)| o == org_id).map(|((_, u), r)| (u.clone(), r.clone())).collect();
            v.sort();
            v
        };
        let mut groups: Vec<GroupRoute> = Vec::new();
        if PLATFORM.contains(&namespace) {
            let org = PLATFORM_ORGS.iter().find(|(ns, _)| *ns == namespace).map(|(_, id)| *id).unwrap_or_default();
            for (u, r) in org_rows(org) {
                if r == "admin" { push(&mut people, &u, "org-admin", Write); }
            }
            for s in supers { push(&mut people, s, "superadmin", Write); }
            groups.push(GroupRoute { group: "everyone".into(), capacity: "everyone", access: access_name(Run) });
            return ProjectRoutes { namespace: namespace.into(), kind: "platform", owner: namespace.into(), private: false, people, groups };
        }
        let Some(p) = self.projects.get(namespace) else {
            for (u, r) in org_rows(LEGACY_ORG) {
                match r.as_str() {
                    "admin" => push(&mut people, &u, "org-admin", Write),
                    "member" => push(&mut people, &u, "org-member", Write),
                    "securityadmin" => push(&mut people, &u, "org-securityadmin", Read),
                    _ => {}
                }
            }
            for s in supers { push(&mut people, s, "superadmin", Write); }
            return ProjectRoutes { namespace: namespace.into(), kind: "legacy", owner: "a3t".into(), private: false, people, groups };
        };
        let by_role = |role: &str| -> Option<(&'static str, NsAccess)> {
            let r = flowr_policy::roles::role("project", role)?;
            let a = match r.access.as_deref()? { "write" => Write, "run" => Run, "read" => Read, _ => return None };
            Some((r.id.as_str(), a))
        };
        let is_member = |u: &str| p.members.iter().any(|(id, _)| id == u);
        for (id, role) in &p.members {
            if let Some((cap, a)) = by_role(role) { push(&mut people, id, cap, a); }
        }
        if p.owner_kind == "user" {
            push(&mut people, &p.owner_id, "owner", Write);
            // A member's role is what they have; SuperAdmin reads a personal
            // project only when it is not private and they are not a member.
            if !p.private {
                for s in supers.iter().filter(|s| !is_member(s)) { push(&mut people, s, "superadmin", Read); }
            }
        } else {
            for (u, r) in org_rows(&p.owner_id) {
                match r.as_str() {
                    "admin" => push(&mut people, &u, "org-admin", Write),
                    "securityadmin" => push(&mut people, &u, "org-securityadmin", Read),
                    _ if !p.private => push(&mut people, &u, "org-member", Read),
                    _ => {}
                }
            }
            for s in supers { push(&mut people, s, "superadmin", Write); }
        }
        let system = p.owner_kind == "org" && p.owner == "system";
        match p.exposure.as_str() {
            "public" if !system => groups.push(GroupRoute { group: "everyone".into(), capacity: "exposed", access: access_name(Run) }),
            "orgs" => {
                for h in p.exposed_to.iter().filter(|h| (!system || h.as_str() == "a3t") && self.orgs.contains_key(h.as_str())) {
                    groups.push(GroupRoute { group: format!("org:{h}"), capacity: "exposed", access: access_name(Run) });
                }
            }
            _ => {}
        }
        ProjectRoutes {
            namespace: namespace.into(),
            kind: if p.owner_kind == "user" { "personal" } else { "org" },
            owner: p.owner.clone(),
            private: p.private,
            people,
            groups,
        }
    }

    /// Every project namespace, and the platform's.
    pub fn namespaces(&self) -> Vec<String> {
        let mut v: Vec<String> = self.projects.keys().cloned().chain(PLATFORM.iter().map(|s| s.to_string())).collect();
        v.sort();
        v
    }

    /// The members (any role) of the org with this handle.
    pub fn members_of(&self, handle: &str) -> Vec<String> {
        let Some(id) = self.orgs.get(handle) else { return Vec::new() };
        let mut v: Vec<String> = self.org_roles.keys().filter(|(o, _)| o == id).map(|(_, u)| u.clone()).collect();
        v.sort();
        v
    }

    /// `user`'s orgs: `(handle, role)`.
    pub fn orgs_of(&self, user: &str) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = self.orgs.iter()
            .filter_map(|(h, id)| self.org_role(id, user).map(|r| (h.clone(), r.to_string()))).collect();
        v.sort();
        v
    }

    /// The projects (namespaces) the org with this handle owns.
    pub fn org_namespaces(&self, handle: &str) -> Vec<String> {
        let mut v: Vec<String> = self.projects.iter()
            .filter(|(_, p)| p.owner_kind == "org" && p.owner == handle)
            .map(|(ns, _)| ns.clone()).collect();
        v.sort();
        v
    }

    /// The orgs whose security `user` looks after: admin or securityadmin there.
    pub fn security_orgs_of(&self, user: &str) -> Vec<String> {
        self.orgs_of(user).into_iter()
            .filter(|(_, r)| r == "admin" || r == "securityadmin").map(|(h, _)| h).collect()
    }

    /// Every org handle.
    pub fn org_handles(&self) -> Vec<String> {
        let mut v: Vec<String> = self.orgs.keys().cloned().collect();
        v.sort();
        v
    }

    /// Everyone the rows name: project owners and members, org members.
    pub fn known_users(&self) -> Vec<String> {
        let mut v: std::collections::BTreeSet<String> = self.org_roles.keys().map(|(_, u)| u.clone()).collect();
        for p in self.projects.values() {
            if p.owner_kind == "user" && !p.owner_id.is_empty() { v.insert(p.owner_id.clone()); }
            v.extend(p.members.iter().map(|(id, _)| id.clone()));
        }
        v.into_iter().collect()
    }
}

/// One way a person reaches a project. `capacity`: `owner`, `developer`,
/// `operator`, `viewer` (named members), `org-admin`, `org-securityadmin`,
/// `org-member` (through the owning org), `superadmin`; `access`: `read`,
/// `run` or `write` (what it lets them do).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Route {
    pub user: String,
    pub capacity: &'static str,
    pub access: &'static str,
}

/// A group reaches a project: `everyone` (signed in), or `org:<handle>` (that
/// org's members). `capacity`: `everyone` (the platform's own namespaces) or
/// `exposed` (the project is shared with them: read, clone, run).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GroupRoute {
    pub group: String,
    pub capacity: &'static str,
    pub access: &'static str,
}

/// Who reaches a project, and how. `kind`: `personal`, `org`, `platform` or
/// `legacy` (a namespace from before projects, org a3t's).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectRoutes {
    pub namespace: String,
    pub kind: &'static str,
    pub owner: String,
    pub private: bool,
    pub people: Vec<Route>,
    pub groups: Vec<GroupRoute>,
}

pub fn access_name(a: NsAccess) -> &'static str {
    match a {
        NsAccess::None => "none",
        NsAccess::Read => "read",
        NsAccess::Run => "run",
        NsAccess::Write => "write",
    }
}

/// Platform SecurityAdmins: Security and the Decision log over every project,
/// no project work. Not a route into a project; listed so people see who audits.
pub const SECURITY_ADMIN_CAPACITY: &str = "security-admin";

/// How far up a capacity sits: 1 reads, 2 runs, 3 builds, 4 manages; 5 audits
/// (security, beside the line rather than above it).
pub fn capacity_rank(capacity: &str) -> u8 {
    // A project role ranks as the role table says (ID-1).
    if let Some(n) = flowr_policy::roles::role("project", capacity).and_then(|r| r.rank) {
        return n;
    }
    match capacity {
        "viewer" | "org-member" | "exposed" | "everyone" => 1,
        "operator" => 2,
        "developer" => 3,
        "owner" | "org-admin" | "superadmin" => 4,
        "org-securityadmin" | SECURITY_ADMIN_CAPACITY => 5,
        _ => 0,
    }
}

/// Whether someone holding `mine` in a project sees a person who holds
/// `theirs` there. Managers (owner, org admin, SuperAdmin) and security (org or
/// platform) see everyone; `infosec` (platform SuperAdmin or SecurityAdmin)
/// too. Others see their own level and below — viewers only viewers, operators
/// operators and viewers — and from developer up also who audits (security).
/// Nobody below a manager sees the managers.
pub fn sees(mine: &[&str], infosec: bool, theirs: &str) -> bool {
    if infosec || mine.iter().any(|c| matches!(capacity_rank(c), 4 | 5)) {
        return true;
    }
    let r = mine.iter().map(|c| capacity_rank(c)).max().unwrap_or(0);
    match capacity_rank(theirs) {
        0 | 4 => false,
        5 => r >= 3,
        t => r >= 1 && t <= r,
    }
}

/// The access snapshot now (refreshed if stale), when the project guard is
/// installed (FlowR EE, enforcing).
pub async fn current_snapshot() -> Option<Arc<Snapshot>> {
    let g = PROJECT_GUARD.get()?;
    Some(g.snapshot().await)
}

/// The org whose decision log holds a decision about `namespace` (AUD-1): an
/// org project's org id; `platform` for platform-wide calls, personal projects,
/// and namespaces this pod doesn't know.
pub async fn audit_org(namespace: Option<&str>) -> String {
    let platform = || "platform".to_string();
    let Some(ns) = namespace.filter(|n| !n.is_empty()) else { return platform() };
    let Some(g) = PROJECT_GUARD.get() else { return platform() };
    let snap = g.snapshot().await;
    match snap.projects.get(ns) {
        Some(p) if p.owner_kind == "org" => snap.orgs.get(&p.owner).cloned()
            .or_else(|| Some(p.owner_id.clone()).filter(|id| !id.is_empty()))
            .unwrap_or_else(platform),
        _ => platform(),
    }
}

/// Every org id this pod knows, for reading all of the decision logs.
pub async fn all_org_ids() -> Vec<String> {
    let Some(g) = PROJECT_GUARD.get() else { return Vec::new() };
    let mut ids: Vec<String> = g.snapshot().await.orgs.values().cloned().collect();
    ids.sort();
    ids.dedup();
    ids
}

impl Snapshot {
    /// Whether this snapshot knows `namespace` as a project.
    pub fn has_project(&self, namespace: &str) -> bool {
        self.projects.contains_key(namespace)
    }
}

#[cfg(test)]
mod route_tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Snapshot {
        Snapshot::from_rows(
            &[
                json!({ "namespace": "ann-orders", "owner_kind": "user", "owner": "ann", "owner_id": "ann", "private": "true", "members": "[{\"id\":\"root\",\"role\":\"viewer\"}]" }),
                json!({ "namespace": "ann-blog", "owner_kind": "user", "owner": "ann", "owner_id": "ann", "private": "false", "members": "[{\"id\":\"bob\",\"role\":\"viewer\"},{\"id\":\"carl\",\"role\":\"operator\"}]" }),
                json!({ "namespace": "acme-web", "owner_kind": "org", "owner": "acme", "owner_id": "org-acme", "private": "true", "members": "[{\"id\":\"carl\",\"role\":\"operator\"},{\"id\":\"dev\",\"role\":\"developer\"}]" }),
                json!({ "namespace": "acme-docs", "owner_kind": "org", "owner": "acme", "owner_id": "org-acme", "private": "false", "members": "[{\"id\":\"zed\",\"role\":\"viewer\"}]",
                        "policy": "{\"exposure\":{\"audience\":\"orgs\",\"orgs\":[\"globex\"]}}" }),
                json!({ "namespace": "acme-pub", "owner_kind": "org", "owner": "acme", "owner_id": "org-acme", "private": "false", "members": "[]",
                        "policy": "{\"exposure\":{\"audience\":\"public\"}}" }),
                json!({ "namespace": "sys-guards", "owner_kind": "org", "owner": "system", "owner_id": "org-system", "private": "true", "members": "[]",
                        "policy": "{\"exposure\":{\"audience\":\"orgs\",\"orgs\":[\"a3t\",\"globex\"]}}" }),
            ],
            &[
                json!({ "org_id": "org-acme", "user_id": "bob", "role": "admin" }),
                json!({ "org_id": "org-acme", "user_id": "carl", "role": "member" }),
                json!({ "org_id": "org-acme", "user_id": "sec", "role": "securityadmin" }),
                json!({ "org_id": "org-globex", "user_id": "gus", "role": "member" }),
                json!({ "org_id": "org-default", "user_id": "dora", "role": "member" }),
                json!({ "org_id": "org-default", "user_id": "ops", "role": "admin" }),
                json!({ "org_id": "org-system", "user_id": "sre", "role": "admin" }),
            ],
            &[json!({ "id": "org-acme", "slug": "acme" }), json!({ "id": "org-globex", "slug": "globex" }),
              json!({ "id": "org-default", "slug": "a3t" }), json!({ "id": "org-system", "slug": "system" })],
        )
    }

    fn rank(a: &str) -> NsAccess {
        match a { "read" => NsAccess::Read, "run" => NsAccess::Run, "write" => NsAccess::Write, _ => NsAccess::None }
    }

    /// What the routes give `user`: their best named route, or a group they are in.
    fn via_routes(s: &Snapshot, r: &ProjectRoutes, user: &str) -> NsAccess {
        let named = r.people.iter().filter(|p| p.user == user).map(|p| rank(p.access)).max().unwrap_or(NsAccess::None);
        let grouped = r.groups.iter().filter(|g| match g.group.strip_prefix("org:") {
            Some(h) => s.members_of(h).iter().any(|m| m == user),
            None => true,
        }).map(|g| rank(g.access)).max().unwrap_or(NsAccess::None);
        named.max(grouped)
    }

    #[test]
    fn routes_agree_with_the_decision_for_everyone() {
        let s = fixture();
        let users = ["ann", "bob", "carl", "sec", "gus", "dora", "ops", "sre", "dev", "zed", "root", "nobody"];
        let mut namespaces = s.namespaces();
        namespaces.push("default".into());
        for ns in &namespaces {
            let r = s.routes(ns, &["root".to_string()]);
            for u in users {
                assert_eq!(via_routes(&s, &r, u), s.decide(u, u == "root", ns), "{u} in {ns}");
            }
        }
    }

    #[test]
    fn routes_say_how_each_person_gets_in() {
        let s = fixture();
        let r = s.routes("acme-web", &["root".to_string()]);
        let has = |u: &str, c: &str| r.people.iter().any(|p| p.user == u && p.capacity == c);
        assert!(has("carl", "operator") && has("dev", "developer") && has("bob", "org-admin") && has("sec", "org-securityadmin") && has("root", "superadmin"));
        assert!(!r.people.iter().any(|p| p.capacity == "org-member"), "private: plain org members are not in");
        let d = s.routes("acme-docs", &[]);
        assert!(d.people.iter().any(|p| p.user == "carl" && p.capacity == "org-member"));
        assert_eq!(d.groups, vec![GroupRoute { group: "org:globex".into(), capacity: "exposed", access: "run" }]);
        assert_eq!(s.routes("sys-guards", &[]).groups.len(), 1, "system exposes to a3t only");
        assert_eq!(s.routes("ann-orders", &["root".to_string()]).people.iter().filter(|p| p.user == "root").count(), 1, "a member's role, not SuperAdmin's read");
    }

    #[test]
    fn an_orgs_projects_and_who_looks_after_its_security() {
        let s = fixture();
        assert_eq!(s.org_namespaces("acme"), vec!["acme-docs", "acme-pub", "acme-web"]);
        assert!(s.org_namespaces("globex").is_empty());
        assert_eq!(s.security_orgs_of("bob"), vec!["acme"]);
        assert_eq!(s.security_orgs_of("sec"), vec!["acme"]);
        assert!(s.security_orgs_of("carl").is_empty());
        assert!(s.security_orgs_of("gus").is_empty());
    }

    #[test]
    fn who_sees_whom() {
        let all = ["viewer", "org-member", "operator", "developer", "owner", "org-admin", "superadmin", "org-securityadmin", "security-admin"];
        let seen = |mine: &[&str]| -> Vec<&str> { all.iter().copied().filter(|t| sees(mine, false, t)).collect() };
        assert_eq!(seen(&["viewer"]), vec!["viewer", "org-member"]);
        assert_eq!(seen(&["operator"]), vec!["viewer", "org-member", "operator"]);
        assert_eq!(seen(&["developer"]), vec!["viewer", "org-member", "operator", "developer", "org-securityadmin", "security-admin"]);
        assert_eq!(seen(&["owner"]).len(), all.len());
        assert_eq!(seen(&["org-securityadmin"]).len(), all.len());
        assert_eq!(seen(&[]), Vec::<&str>::new());
        assert!(sees(&[], true, "superadmin"), "platform InfoSec sees everyone");
    }
}

pub struct ProjectGuard {
    storage: Arc<dyn StorageBackend>,
    snapshot: RwLock<(Option<Instant>, Arc<Snapshot>)>,
    /// When each namespace the snapshot didn't know last forced a re-read. A
    /// namespace seen for the first time re-reads at once (a project made a
    /// moment ago, maybe on another pod); asking again about one that doesn't
    /// exist re-reads at most every UNKNOWN_RETRY, so it can't flood the DB.
    unknown: std::sync::Mutex<HashMap<String, Instant>>,
}

/// Tables whose rows decide project access; a write to one drops the snapshot.
pub const GUARD_TABLES: [&str; 3] = ["projects", "org_members", "orgs"];
const UNKNOWN_RETRY: Duration = Duration::from_secs(2);

/// RT-12: whether `namespace` belongs to an org (or is a project) that runs on
/// its own runtimes only, by this pod's last snapshot. Read synchronously (the
/// router can't wait); a project the snapshot doesn't know yet reads as not.
pub fn isolated(namespace: &str) -> bool {
    if PLATFORM.contains(&namespace) {
        return false;
    }
    let Some(g) = PROJECT_GUARD.get() else { return false };
    let snap = g.snapshot.read().unwrap().1.clone();
    let policy = match snap.projects.get(namespace) {
        Some(p) if p.owner_kind == "org" && p.runtime.is_empty() => snap.org_runtime.get(&p.owner).cloned().unwrap_or_default(),
        Some(p) => p.runtime.clone(),
        None => return false,
    };
    policy == "isolated"
}

/// Forget this pod's access snapshot; the next decision reads the tables again.
pub fn invalidate() {
    if let Some(g) = PROJECT_GUARD.get() {
        g.snapshot.write().unwrap().0 = None;
    }
}

impl ProjectGuard {
    pub fn new(storage: Arc<dyn StorageBackend>) -> Self {
        ProjectGuard { storage, snapshot: RwLock::new((None, Arc::new(Snapshot::default()))), unknown: Default::default() }
    }

    /// Whether to re-read for a namespace the snapshot doesn't know: yes the first
    /// time it is asked about, then at most every UNKNOWN_RETRY.
    fn retry_unknown(&self, ns: &str) -> bool {
        let mut seen = self.unknown.lock().unwrap();
        let now = Instant::now();
        if seen.get(ns).is_some_and(|t| now.duration_since(*t) < UNKNOWN_RETRY) {
            return false;
        }
        if seen.len() > 10_000 {
            seen.retain(|_, t| now.duration_since(*t) < UNKNOWN_RETRY);
        }
        seen.insert(ns.to_string(), now);
        true
    }

    async fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot_for(None).await
    }

    /// The snapshot — refreshed early when it doesn't know `namespace` yet (a
    /// project created a moment ago must not read as a stranger's for 5 s).
    async fn snapshot_for(&self, namespace: Option<&str>) -> Arc<Snapshot> {
        {
            let (at, snap) = &*self.snapshot.read().unwrap();
            let unknown = namespace.filter(|ns| !PLATFORM.contains(ns) && !snap.projects.contains_key(*ns));
            let fresh = at.is_some_and(|t| t.elapsed() < REFRESH) && !unknown.is_some_and(|ns| self.retry_unknown(ns));
            if fresh {
                return snap.clone();
            }
        }
        let db = crate::platform_db::open_platform_db(self.storage.as_ref(), Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
        let rows = |t: &'static str| {
            let db = db.clone();
            async move { db.query(t, None, 100_000).await.map(|b| batches_to_json(&b)) }
        };
        let (p, m, o) = (rows("projects").await, rows("org_members").await, rows("orgs").await);
        let snap = match (p, m, o) {
            (Ok(p), Ok(m), Ok(o)) => Arc::new(Snapshot::from_rows(&p, &m, &o)),
            // Tables not installed yet, or the DB is unreachable: keep the last
            // good snapshot rather than open everything up or lock everyone out
            // on a blip; with none, the empty one (legacy rules only).
            (p, m, o) => {
                tracing::warn!(projects = ?p.err(), members = ?m.err(), orgs = ?o.err(), "project tables unreadable; using the last snapshot");
                self.snapshot.read().unwrap().1.clone()
            }
        };
        *self.snapshot.write().unwrap() = (Some(Instant::now()), snap.clone());
        snap
    }
}

#[async_trait]
impl NamespaceGuard for ProjectGuard {
    async fn access(&self, principal: &Principal, namespace: &str) -> NsAccess {
        if let Principal::Service { .. } = principal {
            return NsAccess::Write; // internal machinery
        }
        let Some(user) = acting_user(principal) else {
            return NsAccess::None; // an agent acting for no one
        };
        let su = is_superadmin(principal);
        let has = self.snapshot_for(Some(namespace)).await.decide(user, su, namespace);
        // Less than write in a project, on a snapshot over a second old: it may
        // predate a change made on another pod (each pod drops only its own
        // snapshot on a write) — re-read once and decide again. At most every
        // UNKNOWN_RETRY per user and project, so refusals can't flood the DB.
        if has < NsAccess::Write && !PLATFORM.contains(&namespace) {
            let old = self.snapshot.read().unwrap().0.is_none_or(|t| t.elapsed() > Duration::from_secs(1));
            if old && self.retry_unknown(&format!("{user}\u{1f}{namespace}")) {
                invalidate();
                return self.snapshot_for(Some(namespace)).await.decide(user, su, namespace);
            }
        }
        has
    }

    fn is_platform(&self, namespace: &str) -> bool {
        PLATFORM.contains(&namespace)
    }

    fn effective_namespace(&self, kind: ItemKind, namespace: &str, name: &str, type_ref: &str, created_by: &str) -> String {
        effective_namespace(kind, namespace, name, type_ref, created_by)
    }

    async fn can_control_runtime(&self, principal: &Principal, labels: &BTreeMap<String, String>) -> bool {
        if matches!(principal, Principal::Service { .. }) || is_superadmin(principal) {
            return true;
        }
        let (Some(org), Some(user)) = (labels.get(ORG_LABEL), acting_user(principal)) else {
            return false; // a shared runtime: superadmin only
        };
        let snap = self.snapshot().await;
        snap.orgs.get(org).is_some_and(|id| snap.org_role(id, user) == Some("admin"))
    }

    async fn can_see_runtime(&self, principal: &Principal, labels: &BTreeMap<String, String>) -> bool {
        let Some(org) = labels.get(ORG_LABEL) else { return true };
        if matches!(principal, Principal::Service { .. }) || is_superadmin(principal) {
            return true;
        }
        let Some(user) = acting_user(principal) else { return false };
        let snap = self.snapshot().await;
        snap.orgs.get(org).is_some_and(|id| snap.org_role(id, user).is_some())
    }

    async fn runtime_allowed(&self, namespace: &str, labels: &BTreeMap<String, String>) -> bool {
        self.snapshot().await.runtime_allowed(namespace, labels)
    }

    fn enforcing(&self) -> bool {
        true
    }
}

/// The runtimes a flow of `namespace` may be placed on (the runtime policy).
pub async fn eligible_runtimes(namespace: &str, flow: &str, runtimes: Vec<flowr_types::canvas::RuntimeSpec>) -> Vec<flowr_types::canvas::RuntimeSpec> {
    let g = flowr_policy::namespace::namespace_guard();
    if !g.enforcing() {
        return runtimes;
    }
    let ns = effective_namespace(ItemKind::Flow, namespace, flow, "", "");
    let mut out = Vec::new();
    for r in runtimes {
        if g.runtime_allowed(&ns, &r.labels).await {
            out.push(r);
        }
    }
    out
}

/// The namespace an item belongs to: where it is stored. Users' agents,
/// templates, canvases and triggers saved in `a3t` before projects count as
/// org `a3t`'s, like the rest of that namespace.
pub fn effective_namespace(_kind: ItemKind, namespace: &str, _name: &str, _type_ref: &str, _created_by: &str) -> String {
    namespace.to_string()
}

/// Install the guard (FlowR EE, policy enforcing).
/// AC-18: whether a new run of `namespace` is admitted: its org's active runs, across its
/// projects, are under `settings.quota.max_active_runs`. An org with no quota, and the
/// platform's own namespaces, always are. A failed count admits: the quota protects
/// neighbours, and it must not stop an org because the count couldn't be read.
pub async fn admit(storage: &dyn StorageBackend, namespace: &str) -> Result<(), String> {
    if PLATFORM.contains(&namespace) || namespace == crate::router::GATEWAY_NAMESPACE {
        return Ok(());
    }
    let snap = guard_snapshot(namespace).await;
    let Some(snap) = snap else { return Ok(()) };
    let Some(project) = snap.projects.get(namespace).filter(|p| p.owner_kind == "org") else { return Ok(()) };
    let Some(&max) = snap.org_max_runs.get(&project.owner) else { return Ok(()) };
    let namespaces: Vec<String> = snap.projects.iter().filter(|(_, p)| p.owner_kind == "org" && p.owner == project.owner).map(|(n, _)| n.clone()).collect();
    let Some(engine) = storage.engine() else { return Ok(()) };
    use flowr_storage::engine::{RunPhase, RunQuery};
    let q = RunQuery { namespaces: Some(namespaces), phases: vec![RunPhase::Pending, RunPhase::Running, RunPhase::Waiting], limit: max + 1, ..Default::default() };
    match engine.list(q).await {
        Ok(page) if page.items.len() >= max => Err(format!("org_quota: {} has reached its limit of {max} active runs. Try again when some finish, or ask an admin to raise it.", project.owner)),
        _ => Ok(()),
    }
}

/// The access snapshot, when the project guard is installed.
async fn guard_snapshot(namespace: &str) -> Option<Arc<Snapshot>> {
    let g = PROJECT_GUARD.get()?;
    Some(g.snapshot_for(Some(namespace)).await)
}

pub fn install(storage: Arc<dyn StorageBackend>) {
    let for_admission = Arc::clone(&storage);
    flowr_mcp_core::flow_base::set_admission(Arc::new(move |namespace: String| {
        let storage = Arc::clone(&for_admission);
        Box::pin(async move { admit(storage.as_ref(), &namespace).await })
    }));
    let guard = Arc::new(ProjectGuard::new(storage));
    let _ = PROJECT_GUARD.set(Arc::clone(&guard));
    flowr_policy::namespace::set_namespace_guard(guard);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snap() -> Snapshot {
        Snapshot::from_rows(
            &[
                json!({ "namespace": "ann-orders", "owner_kind": "user", "owner_id": "ann", "private": "true", "members": "[]" }),
                json!({ "namespace": "ann-blog", "owner_kind": "user", "owner_id": "ann", "private": "false", "members": "[{\"id\":\"bob\",\"role\":\"viewer\"}]" }),
                json!({ "namespace": "acme-web", "owner_kind": "org", "owner_id": "org-acme", "private": "true", "members": "[{\"id\":\"carl\",\"role\":\"operator\"}]" }),
                json!({ "namespace": "acme-docs", "owner_kind": "org", "owner_id": "org-acme", "private": "false", "members": "[]" }),
                json!({ "namespace": "globex-x", "owner_kind": "org", "owner_id": "org-globex", "private": "false", "members": "[]" }),
            ],
            &[
                json!({ "org_id": "org-acme", "user_id": "bob", "role": "admin" }),
                json!({ "org_id": "org-acme", "user_id": "carl", "role": "member" }),
                json!({ "org_id": "org-acme", "user_id": "sec", "role": "securityadmin" }),
                json!({ "org_id": "org-default", "user_id": "dora", "role": "member" }),
            ],
            &[json!({ "id": "org-acme", "slug": "acme" })],
        )
    }

    #[test]
    fn exposed_projects_let_their_audience_read_clone_and_run() {
        let s = Snapshot::from_rows(
            &[
                json!({ "namespace": "system-guards", "owner_kind": "org", "owner": "system", "owner_id": "org-system", "private": "true", "members": "[]",
                        "policy": "{\"exposure\":{\"audience\":\"orgs\",\"orgs\":[\"a3t\"],\"kind\":\"shared\"}}" }),
                json!({ "namespace": "a3t-training", "owner_kind": "org", "owner": "a3t", "owner_id": "org-default", "private": "false", "members": "[]",
                        "policy": "{\"exposure\":{\"audience\":\"public\",\"kind\":\"training\"}}" }),
                json!({ "namespace": "acme-community", "owner_kind": "org", "owner": "acme", "owner_id": "org-acme", "private": "false", "members": "[]",
                        "policy": "{\"exposure\":{\"audience\":\"public\"}}" }),
            ],
            &[
                json!({ "org_id": "org-default", "user_id": "ops", "role": "member" }),
                json!({ "org_id": "org-acme", "user_id": "bob", "role": "admin" }),
            ],
            &[json!({ "id": "org-default", "slug": "a3t" }), json!({ "id": "org-acme", "slug": "acme" }), json!({ "id": "org-system", "slug": "system" })],
        );
        // system's guards: exposed to org a3t only.
        assert_eq!(s.decide("ops", false, "system-guards"), NsAccess::Run);
        assert_eq!(s.decide("bob", false, "system-guards"), NsAccess::None);
        // system can't go public, nor to another org than a3t.
        let leaky = Snapshot::from_rows(
            &[json!({ "namespace": "system-leak", "owner_kind": "org", "owner": "system", "owner_id": "org-system", "private": "false", "members": "[]",
                      "policy": "{\"exposure\":{\"audience\":\"public\"}}" }),
              json!({ "namespace": "system-acme", "owner_kind": "org", "owner": "system", "owner_id": "org-system", "private": "false", "members": "[]",
                      "policy": "{\"exposure\":{\"audience\":\"orgs\",\"orgs\":[\"acme\"]}}" })],
            &[json!({ "org_id": "org-acme", "user_id": "bob", "role": "admin" })],
            &[json!({ "id": "org-acme", "slug": "acme" })],
        );
        assert_eq!(leaky.decide("bob", false, "system-leak"), NsAccess::None);
        assert_eq!(leaky.decide("bob", false, "system-acme"), NsAccess::None);
        // a3t's training and acme's community: everyone runs, nobody else changes.
        assert_eq!(s.decide("bob", false, "a3t-training"), NsAccess::Run);
        assert_eq!(s.decide("ops", false, "acme-community"), NsAccess::Run);
        assert_eq!(s.decide("bob", false, "acme-community"), NsAccess::Write, "its owner still writes");
    }

    #[test]
    fn platform_org_admins_change_their_platform_namespace() {
        let s = Snapshot::from_rows(&[], &[
            json!({ "org_id": "org-default", "user_id": "ops", "role": "admin" }),
            json!({ "org_id": "org-system", "user_id": "sre", "role": "admin" }),
        ], &[]);
        assert_eq!(s.decide("ops", false, "a3t"), NsAccess::Write);
        assert_eq!(s.decide("ops", false, "system"), NsAccess::Run);
        assert_eq!(s.decide("sre", false, "system"), NsAccess::Write);
        assert_eq!(s.decide("sre", false, "a3t"), NsAccess::Run);
    }

    #[test]
    fn an_org_admin_sees_only_their_org() {
        let s = snap();
        assert_eq!(s.decide("bob", false, "acme-web"), NsAccess::Write);
        assert_eq!(s.decide("bob", false, "acme-docs"), NsAccess::Write);
        assert_eq!(s.decide("bob", false, "globex-x"), NsAccess::None, "another org's project");
        assert_eq!(s.decide("bob", false, "ann-orders"), NsAccess::None, "a private personal project");
        assert_eq!(s.decide("bob", false, "ann-blog"), NsAccess::Read, "a member, as viewer");
        assert_eq!(s.decide("bob", false, "default"), NsAccess::None, "not in org default");
    }

    #[test]
    fn org_members_security_admins_and_private_projects() {
        let s = snap();
        assert_eq!(s.decide("carl", false, "acme-docs"), NsAccess::Read);
        assert_eq!(s.decide("carl", false, "acme-web"), NsAccess::Run, "private, but a member as operator");
        assert_eq!(s.decide("sec", false, "acme-web"), NsAccess::Read);
        assert_eq!(s.decide("zed", false, "acme-docs"), NsAccess::None);
    }

    #[test]
    fn superadmin_platform_and_legacy() {
        let s = snap();
        assert_eq!(s.decide("root", true, "globex-x"), NsAccess::Write);
        assert_eq!(s.decide("root", true, "ann-orders"), NsAccess::None, "private personal: owner only");
        assert_eq!(s.decide("root", true, "ann-blog"), NsAccess::Read);
        assert_eq!(s.decide("zed", false, "a3t"), NsAccess::Run);
        assert_eq!(s.decide("root", true, "a3t"), NsAccess::Write);
        assert_eq!(s.decide("dora", false, "default"), NsAccess::Write);
        assert_eq!(s.decide("dora", false, "a3t"), NsAccess::Run, "a member of org a3t runs its flows");
        assert_eq!(s.decide("ann", false, "ann-orders"), NsAccess::Write);
    }

    fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn runtime_policy_keeps_isolated_work_on_its_own_runtimes() {
        let s = Snapshot::from_rows(
            &[
                json!({ "namespace": "acme-web", "owner_kind": "org", "owner": "acme", "owner_id": "org-acme", "private": "true", "members": "[]", "policy": "{}" }),
                json!({ "namespace": "acme-lab", "owner_kind": "org", "owner": "acme", "owner_id": "org-acme", "private": "false", "members": "[]", "policy": "{\"runtime\":\"shared\"}" }),
                json!({ "namespace": "globex-x", "owner_kind": "org", "owner": "globex", "owner_id": "org-globex", "private": "false", "members": "[]", "policy": "{}" }),
                json!({ "namespace": "ann-secret", "owner_kind": "user", "owner": "ann", "owner_id": "ann", "private": "true", "members": "[]", "policy": "{\"runtime\":\"isolated\"}" }),
                json!({ "namespace": "ann-blog", "owner_kind": "user", "owner": "ann", "owner_id": "ann", "private": "false", "members": "[]", "policy": "{}" }),
            ],
            &[],
            &[json!({ "id": "org-acme", "slug": "acme", "settings": "{\"runtime\":\"isolated\"}" }),
              json!({ "id": "org-globex", "slug": "globex", "settings": "{}" })],
        );
        let shared = labels(&[]);
        let acme = labels(&[(ORG_LABEL, "acme")]);
        let globex = labels(&[(ORG_LABEL, "globex")]);
        let ann = labels(&[(PROJECT_LABEL, "ann-secret")]);
        // acme is isolated: only its own runtime, never shared or globex's.
        assert!(s.runtime_allowed("acme-web", &acme));
        assert!(!s.runtime_allowed("acme-web", &shared));
        assert!(!s.runtime_allowed("acme-web", &globex));
        // ...unless the project chose shared.
        assert!(s.runtime_allowed("acme-lab", &shared));
        assert!(!s.runtime_allowed("acme-lab", &globex));
        // globex is shared: shared runtimes or its own, never acme's.
        assert!(s.runtime_allowed("globex-x", &shared));
        assert!(s.runtime_allowed("globex-x", &globex));
        assert!(!s.runtime_allowed("globex-x", &acme));
        // An isolated personal project: its own runtime only.
        assert!(s.runtime_allowed("ann-secret", &ann));
        assert!(!s.runtime_allowed("ann-secret", &shared));
        assert!(!s.runtime_allowed("ann-blog", &ann), "another project's runtime");
        assert!(s.runtime_allowed("ann-blog", &shared));
        // The platform's flows: shared runtimes only.
        assert!(s.runtime_allowed("a3t", &shared));
        assert!(!s.runtime_allowed("a3t", &acme));
    }

    #[test]
    fn items_belong_where_they_are_stored() {
        assert_eq!(effective_namespace(ItemKind::Flow, "a3t", "ops-ping", "", ""), "a3t");
        assert_eq!(effective_namespace(ItemKind::Agent, "a3t", "x", "", "usr-1"), "a3t");
        assert_eq!(effective_namespace(ItemKind::Flow, "acme-web", "a3t-x", "", ""), "acme-web");
    }
}

// ── Enforcement in FlowR CE's own tools and routes ──

use flowr_mcp_core::CallToolResult;

fn arg<'a>(args: &'a Value, k: &str) -> &'a str {
    args.get(k).and_then(|v| v.as_str()).unwrap_or_default()
}

fn guard() -> Arc<dyn NamespaceGuard> {
    flowr_policy::namespace::namespace_guard()
}

/// The namespace a deployment's work belongs to.
/// The project a deployment belongs to: an agent's is the agent's own project
/// (its record's namespace) — not its serving flow's, which agents share (a3t's
/// tmpl_a3t_agent); a flow's is the flow's.
async fn deployment_namespace(d: &crate::ops::Deployment) -> String {
    if d.kind == "agent" {
        if let Some(storage) = crate::agent_keys::store() {
            let id = if d.agent_ref.is_empty() { d.target.as_str() } else { d.agent_ref.rsplit('/').next().unwrap_or(&d.target) };
            let row = crate::handlers::call_lancedb::query_platform_table(
                storage.as_ref(), "agent_configs",
                Some(format!("id = {}", crate::handlers::call_lancedb::sql_quote(id))), 1).await
                .ok().and_then(|r| r.into_iter().next());
            if let Some(row) = row {
                let ns = row.get("namespace").and_then(|v| v.as_str()).unwrap_or("a3t");
                return effective_namespace(ItemKind::Agent, ns, id, "", row.get("created_by").and_then(|v| v.as_str()).unwrap_or_default());
            }
        }
    }
    let ns = d.flow_ref.split_once('/').map(|(n, _)| n.to_string()).unwrap_or_else(|| d.namespace.clone());
    let name = d.flow_ref.split_once('/').map(|(_, n)| n).unwrap_or(&d.target);
    effective_namespace(ItemKind::Flow, &ns, name, "", "")
}

async fn allowed(principal: &Principal, ns: &str, needed: NsAccess) -> Result<(), CallToolResult> {
    if needed == NsAccess::Read && flowr_policy::namespace::platform_hidden(principal, ns) {
        flowr_mcp_core::ns_gate::refused(principal, ns, "a call", needed, NsAccess::None);
        return Err(CallToolResult::error(format!("Not found: no project '{ns}' that you can see.")));
    }
    let has = guard().access(principal, ns).await;
    if has >= needed {
        return Ok(());
    }
    flowr_mcp_core::ns_gate::refused(principal, ns, "a call", needed, has);
    Err(CallToolResult::error(if has == NsAccess::None {
        format!("Not found: no project '{ns}' that you can see.")
    } else {
        format!("Forbidden: this needs more access to project '{ns}' than you have.")
    }))
}

/// Gate FlowR CE's own tools (agents, canvases, runtimes, deployments, the DB
/// console). `Some` = refused.
pub async fn gate_ce_tool(storage: &Arc<dyn StorageBackend>, principal: &Principal, name: &str, args: &Value) -> Option<CallToolResult> {
    if !guard().enforcing() {
        return None;
    }
    let ns_or = |d: &str| { let n = arg(args, "namespace"); if n.is_empty() { d.to_string() } else { n.to_string() } };
    let canvas = |level| {
        let ns = ns_or("a3t");
        (effective_namespace(ItemKind::Resource, &ns, arg(args, "name"), "canvas", ""), level)
    };
    let need: Option<(String, NsAccess)> = match name {
        "get_canvas" | "preview_placement" => Some(canvas(NsAccess::Read)),
        "update_canvas" | "bind_canvas" => Some(canvas(NsAccess::Write)),
        "clone_canvas" => Some((ns_or("default"), NsAccess::Write)),
        "deploy_flow" => {
            let ns = ns_or("a3t");
            Some((effective_namespace(ItemKind::Flow, &ns, arg(args, "name"), "", ""), NsAccess::Write))
        }
        "create_agent" => Some((ns_or("default"), NsAccess::Write)),
        "get_agent" | "delete_agent" | "send_message" => {
            let id = arg(args, "agent_id");
            let row = crate::handlers::call_lancedb::query_platform_table(
                storage.as_ref(), "agent_configs",
                Some(format!("id = {}", crate::handlers::call_lancedb::sql_quote(id))), 1).await
                .ok().and_then(|r| r.into_iter().next()).unwrap_or(Value::Null);
            let ns = effective_namespace(ItemKind::Agent, row.get("namespace").and_then(|v| v.as_str()).unwrap_or("a3t"),
                id, "", row.get("created_by").and_then(|v| v.as_str()).unwrap_or_default());
            let level = match name { "get_agent" => NsAccess::Read, "send_message" => NsAccess::Run, _ => NsAccess::Write };
            Some((ns, level))
        }
        "query_resource" => {
            let ns = ns_or("default");
            let r = arg(args, "resource");
            let type_ref = storage.resources().get(&ns, r).await.map(|x| x.type_ref).unwrap_or_default();
            Some((effective_namespace(ItemKind::Resource, &ns, r, &type_ref, ""), NsAccess::Read))
        }
        "probe_deployment" | "pause_deployment" | "resume_deployment" | "restart_deployment" | "undeploy" | "terminate_deployment" => {
            let Some(d) = crate::ops::deployment_by_id(storage.as_ref(), arg(args, "deployment")).await else { return None };
            let level = if name == "probe_deployment" { NsAccess::Read } else { NsAccess::Write };
            Some((deployment_namespace(&d).await, level))
        }
        "terminate_runtime" | "set_runtime_state" | "set_paused_response" => {
            let rt = crate::ops::runtime_by_name(storage.as_ref(), arg(args, "runtime")).await;
            let labels = rt.as_ref().map(|r| r.labels.clone()).unwrap_or_default();
            let rt_ns = rt.as_ref().map(|r| r.namespace.clone()).unwrap_or_default();
            if !guard().can_see_runtime(principal, &labels).await {
                return Some(CallToolResult::error("Not found: no such runtime.".to_string()));
            }
            if !controls_runtime(principal, &rt_ns, &labels).await {
                return Some(CallToolResult::error(
                    "Forbidden: a shared runtime is controlled by superadmin only; an org's own runtime by its admins; a project's own runtime by whoever may change that project.".to_string()));
            }
            None
        }
        "create_runtime" => {
            let labels: BTreeMap<String, String> = args.get("labels").and_then(|l| l.as_object())
                .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect())
                .unwrap_or_default();
            if !guard().can_control_runtime(principal, &labels).await {
                return Some(CallToolResult::error(
                    "Forbidden: shared runtimes are created by superadmin; an org's own runtime (label a3t.io/org) by its admins.".to_string()));
            }
            None
        }
        _ => None,
    };
    let (ns, level) = need?;
    allowed(principal, &ns, level).await.err()
}

/// The runtime overview (`list_deployments`, `GET /ops/overview`) as `principal`
/// may see it: runtimes it may see (shared ones, its orgs' own), and on each only
/// the deployments of projects it can read.
pub async fn filter_overview(principal: &Principal, mut overview: Value) -> Value {
    let g = guard();
    if !g.enforcing() {
        return overview;
    }
    async fn visible_deployment(p: &Principal, d: &Value) -> bool {
        let Ok(dep) = serde_json::from_value::<crate::ops::Deployment>(d.get("deployment").cloned().unwrap_or(Value::Null)) else { return false };
        guard().access(p, &deployment_namespace(&dep).await).await >= NsAccess::Read
    }
    if let Some(rts) = overview.get_mut("runtimes").and_then(|v| v.as_array_mut()) {
        let mut keep = Vec::new();
        for mut rt in std::mem::take(rts) {
            let labels: BTreeMap<String, String> = rt.get("runtime").and_then(|r| r.get("labels"))
                .and_then(|l| serde_json::from_value(l.clone()).ok()).unwrap_or_default();
            if !g.can_see_runtime(principal, &labels).await {
                continue;
            }
            let rt_ns = rt.get("runtime").and_then(|r| r.get("namespace")).and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let control = controls_runtime(principal, &rt_ns, &labels).await;
            if let Some(deps) = rt.get_mut("deployments").and_then(|v| v.as_array_mut()) {
                let mut shown = Vec::new();
                for d in std::mem::take(deps) {
                    if visible_deployment(principal, &d).await { shown.push(d); }
                }
                *deps = shown;
            }
            rt["canControl"] = Value::Bool(control);
            keep.push(rt);
        }
        *rts = keep;
    }
    if let Some(orphans) = overview.get_mut("unplaced").and_then(|v| v.as_array_mut()) {
        let mut shown = Vec::new();
        for d in std::mem::take(orphans) {
            if visible_deployment(principal, &d).await { shown.push(d); }
        }
        *orphans = shown;
    }
    overview
}

/// Runtime pools `principal` may see.
pub async fn filter_runtimes(principal: &Principal, runtimes: Vec<flowr_types::canvas::RuntimeSpec>) -> Vec<flowr_types::canvas::RuntimeSpec> {
    let g = guard();
    let mut out = Vec::new();
    for r in runtimes {
        if !g.enforcing() || g.can_see_runtime(principal, &r.labels).await {
            out.push(r);
        }
    }
    out
}

/// Rows the caller may see, for flows (`call: registry` operation `visible`):
/// each item's `namespace` (and `name`, `type`, `created_by`) decide.
pub async fn filter_items(principal: &Principal, kind: ItemKind, items: Vec<Value>) -> Vec<Value> {
    let g = guard();
    if !g.enforcing() {
        return items;
    }
    let mut out = Vec::new();
    for it in items {
        let s = |k: &str| it.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let ns = if s("namespace").is_empty() { "default".to_string() } else { s("namespace") };
        let ns = effective_namespace(kind, &ns, &s("name"), &s("type"), &s("created_by"));
        if !flowr_policy::namespace::platform_hidden(principal, &ns) && g.access(principal, &ns).await >= NsAccess::Read {
            out.push(it);
        }
    }
    out
}

/// The text of a tool result (for an HTTP error body).
pub fn result_text(r: &CallToolResult) -> String {
    r.content.iter().map(|c| match c { flowr_mcp_core::protocol::ToolContent::Text { text } => text.clone(), #[allow(unreachable_patterns)] _ => String::new() })
        .collect::<Vec<_>>().join(" ")
}

/// Whether `principal` may see an item (REST routes).
pub async fn can_read_item(principal: &Principal, kind: ItemKind, ns: &str, name: &str, type_ref: &str) -> bool {
    let g = guard();
    if !g.enforcing() {
        return true;
    }
    let ns = effective_namespace(kind, ns, name, type_ref, "");
    !flowr_policy::namespace::platform_hidden(principal, &ns) && g.access(principal, &ns).await >= NsAccess::Read
}

/// A REST write into `ns`: `Err` with the response when refused.
/// Who controls a runtime (pause, resume, restart, terminate): as the guard says
/// (superadmin for a shared one, an org's admins for the org's own) — and, for a
/// runtime made in a project (not a platform namespace, no org label), whoever
/// may change that project.
async fn controls_runtime(principal: &Principal, ns: &str, labels: &BTreeMap<String, String>) -> bool {
    let g = guard();
    if g.can_control_runtime(principal, labels).await {
        return true;
    }
    !ns.is_empty() && !["system", "a3t", "default"].contains(&ns) && !labels.contains_key(ORG_LABEL)
        && g.access(principal, ns).await >= NsAccess::Write
}

pub async fn rest_write(principal: &Principal, ns: &str) -> Result<(), (axum::http::StatusCode, axum::Json<Value>)> {
    let g = guard();
    if !g.enforcing() {
        return Ok(());
    }
    let has = g.access(principal, ns).await;
    if has != NsAccess::Write {
        flowr_mcp_core::ns_gate::refused(principal, ns, "a change", NsAccess::Write, has);
    }
    match has {
        NsAccess::Write => Ok(()),
        NsAccess::None => Err((axum::http::StatusCode::NOT_FOUND, axum::Json(serde_json::json!({ "error": format!("no project '{ns}' that you can see") })))),
        // Not "changing": some of these are reads only writers may make (an
        // agent's keys), so say what is needed and what the caller has.
        has => {
            let level = if has == NsAccess::Run { "run" } else { "read" };
            Err((axum::http::StatusCode::FORBIDDEN, axum::Json(serde_json::json!({ "error": format!("this needs write access to project '{ns}'; you have {level}") }))))
        }
    }
}
