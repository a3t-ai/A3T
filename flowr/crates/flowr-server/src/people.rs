//! Who can reach a project, and how — and everyone's access, for an audit.
//!
//! - `GET /projects/:project/people` — the people who can reach a project and
//!   the route each takes (named member, the owning org's admin / security
//!   admin / member, owner, SuperAdmin), plus the groups too wide to name
//!   (everyone signed in, another org through exposure). Computed from the same
//!   rows and rules the server enforces (`flowr_ce::access`). Anyone who can
//!   see the project may ask; what they get back depends on where they stand in
//!   it (`flowr_ce::access::sees`): viewers see viewers, developers see who
//!   builds, runs and reads plus who audits, managers and security see all.
//! - `GET /security/people` — every person (the sign-in realm, and anyone the
//!   projects and orgs name), their platform roles, orgs and project access.
//!   SuperAdmin and SecurityAdmin only.
//!
//! Platform roles live in the IdP: read in bulk per realm role (no call per
//! person), so hundreds of people cost a handful of requests.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use flowr_ce::access::{self, capacity_rank, ProjectRoutes, Route, SECURITY_ADMIN_CAPACITY};
use flowr_policy::{namespace::NsAccess, roles, Principal};

fn refuse(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// A person as the IdP knows them.
#[derive(Debug, Clone, Default)]
struct Person {
    email: String,
    name: String,
    enabled: bool,
}

/// The sign-in realm: its people, and who holds each A3T role.
#[derive(Debug, Default)]
struct Directory {
    people: HashMap<String, Person>,
    /// user id → A3T roles (canonical: SuperAdmin, SecurityAdmin, Admin, …).
    roles: HashMap<String, BTreeSet<String>>,
    /// user id → the realm roles they were given that mean an A3T role
    /// (`superadmin`, `admin`, …: what Identity › Users › Edit sets).
    realm: HashMap<String, BTreeSet<String>>,
    /// Why the realm could not be read (people then show by id).
    error: Option<String>,
}

impl Directory {
    fn holds(&self, user: &str, role: &str) -> bool {
        self.roles.get(user).is_some_and(|r| r.contains(role))
    }
    fn with(&self, role: &str) -> Vec<String> {
        let mut v: Vec<String> = self.roles.iter().filter(|(_, r)| r.contains(role)).map(|(u, _)| u.clone()).collect();
        v.sort();
        v
    }
}

#[cfg(feature = "ee")]
async fn kc_pages(client: &reqwest::Client, token: &str, url: &str) -> Result<Vec<Value>, String> {
    const PAGE: usize = 500;
    let mut out = Vec::new();
    for first in (0..).step_by(PAGE) {
        let sep = if url.contains('?') { '&' } else { '?' };
        let resp = client.get(format!("{url}{sep}first={first}&max={PAGE}")).bearer_auth(token).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("the sign-in realm answered {}", resp.status()));
        }
        let page: Vec<Value> = resp.json().await.map_err(|e| e.to_string())?;
        let n = page.len();
        out.extend(page);
        if n < PAGE || first > 100_000 {
            break;
        }
    }
    Ok(out)
}

/// Read the realm: its people, and the members of each realm role that means an
/// A3T role. `all_roles`: every role (the audit); else only SuperAdmin and
/// SecurityAdmin (all a project's list needs).
async fn directory(idp: &crate::IdpAdminState, all_roles: bool) -> Directory {
    #[cfg(feature = "ee")]
    {
        use aesop_tower::oidc_admin as kc;
        let mut d = Directory::default();
        let Some(cfg) = kc::IdpAdminConfig::from_issuer(&idp.issuer, idp.admin_user.clone(), idp.admin_password.clone()) else {
            d.error = Some("no sign-in realm is configured".into());
            return d;
        };
        let token = match kc::admin_token(&idp.client, &cfg).await {
            Ok(t) => t,
            Err(e) => { d.error = Some(e.to_string()); return d; }
        };
        let base = format!("{}/admin/realms/{}", cfg.base_url, cfg.realm);
        match kc_pages(&idp.client, &token, &format!("{base}/users?briefRepresentation=true")).await {
            Ok(users) => {
                let s = |u: &Value, k: &str| u.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
                for u in &users {
                    let name = [s(u, "firstName"), s(u, "lastName")].join(" ").trim().to_string();
                    d.people.insert(s(u, "id"), Person {
                        email: if s(u, "email").is_empty() { s(u, "username") } else { s(u, "email") },
                        name,
                        enabled: u.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false),
                    });
                }
            }
            Err(e) => { d.error = Some(e); return d; }
        }
        let realm_roles: Vec<String> = match idp.client.get(format!("{base}/roles")).bearer_auth(&token).send().await {
            Ok(r) if r.status().is_success() => r.json::<Vec<Value>>().await.unwrap_or_default().iter()
                .filter_map(|r| r.get("name").and_then(|n| n.as_str()).map(String::from)).collect(),
            Ok(r) => { d.error = Some(format!("realm roles: {}", r.status())); Vec::new() }
            Err(e) => { d.error = Some(e.to_string()); Vec::new() }
        };
        for role in realm_roles {
            // Only roles that mean an A3T role (not the IdP's own, like default-roles-<realm>).
            let a3t: Vec<String> = roles::canonicalize(&role).into_iter().filter(|r| roles::ALL.contains(&r.as_str())).collect();
            let wanted = all_roles || a3t.iter().any(|r| r == roles::SUPER_ADMIN || r == roles::SECURITY_ADMIN);
            if a3t.is_empty() || !wanted {
                continue;
            }
            let enc: String = role.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
            match kc_pages(&idp.client, &token, &format!("{base}/roles/{enc}/users?briefRepresentation=true")).await {
                Ok(members) => {
                    for m in members {
                        if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
                            d.roles.entry(id.to_string()).or_default().extend(a3t.iter().cloned());
                            d.realm.entry(id.to_string()).or_default().insert(role.to_ascii_lowercase());
                        }
                    }
                }
                Err(e) => d.error = Some(format!("members of {role}: {e}")),
            }
        }
        d
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = (idp, all_roles);
        Directory { error: Some("the sign-in realm is read by FlowR EE".into()), ..Default::default() }
    }
}

/// A person by id: the realm's record, else what the project and org rows say.
fn person(dir: &Directory, snap: &access::Snapshot, user: &str) -> Person {
    dir.people.get(user).cloned().unwrap_or_else(|| {
        let (email, name) = snap.contact(user).unwrap_or_default();
        Person { email, name, enabled: false }
    })
}

fn human(principal: Option<Extension<Principal>>) -> Option<(String, Vec<String>)> {
    match principal {
        Some(Extension(Principal::Human { user_id, roles, .. })) => Some((user_id, roles)),
        _ => None,
    }
}

fn infosec(roles: &[String]) -> bool {
    roles.iter().any(|r| r == roles::SUPER_ADMIN || r == roles::SECURITY_ADMIN)
}

fn access_rank(a: &str) -> u8 {
    match a { "write" => 4, "run" => 3, "read" => 2, "audit" => 1, _ => 0 }
}

/// A project's routes, with the platform SecurityAdmins (who audit every
/// project, and do no project work there) added.
fn routes_with_audit(snap: &access::Snapshot, ns: &str, dir: &Directory) -> ProjectRoutes {
    let supers = dir.with(roles::SUPER_ADMIN);
    let mut r = snap.routes(ns, &supers);
    for u in dir.with(roles::SECURITY_ADMIN) {
        if !dir.holds(&u, roles::SUPER_ADMIN) {
            r.people.push(Route { user: u, capacity: SECURITY_ADMIN_CAPACITY, access: "audit" });
        }
    }
    r
}

/// One row per person: their best access and every capacity they hold.
fn by_person(routes: &[Route]) -> BTreeMap<String, (String, Vec<&'static str>)> {
    let mut m: BTreeMap<String, (String, Vec<&'static str>)> = BTreeMap::new();
    for r in routes {
        let e = m.entry(r.user.clone()).or_insert_with(|| ("none".into(), Vec::new()));
        if access_rank(r.access) > access_rank(&e.0) {
            e.0 = r.access.to_string();
        }
        if !e.1.contains(&r.capacity) {
            e.1.push(r.capacity);
        }
    }
    for (_, caps) in m.values_mut() {
        caps.sort_by_key(|c| std::cmp::Reverse(capacity_rank(c)));
    }
    m
}

#[derive(serde::Deserialize, Default)]
pub struct PeopleQuery {
    /// Part of a name or email.
    #[serde(default)]
    q: Option<String>,
    /// Only people holding this capacity (`developer`, `org-admin`, …).
    #[serde(default)]
    capacity: Option<String>,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

/// `GET /projects/:project/people`.
pub async fn project_people_handler(
    Extension(idp): Extension<crate::IdpAdminState>,
    principal: Option<Extension<Principal>>,
    Path(ns): Path<String>,
    Query(q): Query<PeopleQuery>,
) -> Response {
    let Some((me, my_roles)) = human(principal) else {
        return refuse(StatusCode::UNAUTHORIZED, "Sign in first.");
    };
    let is_infosec = infosec(&my_roles);
    let Some(snap) = access::current_snapshot().await else {
        return refuse(StatusCode::NOT_IMPLEMENTED, "Who can reach a project is known when project access is enforced (FlowR EE).");
    };
    let principal = Principal::Human { user_id: me.clone(), roles: my_roles.clone(), memory_branch: None, impersonated_by: None };
    let mine = flowr_policy::namespace::namespace_guard().access(&principal, &ns).await;
    if mine == NsAccess::None && !is_infosec {
        return refuse(StatusCode::NOT_FOUND, &format!("No project {ns}."));
    }
    let dir = directory(&idp, false).await;
    let routes = routes_with_audit(&snap, &ns, &dir);
    let people = by_person(&routes.people);
    let my_caps: Vec<&str> = people.get(&me).map(|(_, c)| c.clone()).unwrap_or_default();
    let sees_all = is_infosec || my_caps.iter().any(|c| matches!(capacity_rank(c), 4 | 5));
    let needle = q.q.as_deref().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let mut rows: Vec<Value> = people.iter()
        .filter(|(u, (_, caps))| **u == me || caps.iter().all(|c| access::sees(&my_caps, is_infosec, c)))
        .filter(|(_, (_, caps))| q.capacity.as_deref().is_none_or(|want| caps.iter().any(|c| *c == want)))
        .map(|(u, (best, caps))| {
            let p = person(&dir, &snap, u);
            json!({ "id": u, "email": p.email, "name": p.name, "account": dir.people.contains_key(u),
                    "access": best, "capacities": caps, "you": *u == me })
        })
        .filter(|v| needle.as_ref().is_none_or(|n| {
            let f = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_lowercase().contains(n.as_str());
            f("email") || f("name") || f("id")
        }))
        .collect();
    rows.sort_by(|a, b| {
        let k = |v: &Value| (std::cmp::Reverse(access_rank(v["access"].as_str().unwrap_or_default())),
            std::cmp::Reverse(v["capacities"].get(0).and_then(|c| c.as_str()).map(capacity_rank).unwrap_or(0)),
            v["name"].as_str().filter(|s| !s.is_empty()).or(v["email"].as_str()).unwrap_or_default().to_lowercase());
        k(a).cmp(&k(b))
    });
    let total = rows.len();
    let (offset, limit) = (q.offset.unwrap_or(0), q.limit.unwrap_or(500).min(5000));
    let page: Vec<Value> = rows.into_iter().skip(offset).take(limit).collect();
    let groups: Vec<Value> = routes.groups.iter().map(|g| json!({ "group": g.group, "capacity": g.capacity, "access": g.access })).collect();
    Json(json!({
        "namespace": routes.namespace, "kind": routes.kind, "owner": routes.owner, "private": routes.private,
        "you": { "capacities": my_caps, "access": access::access_name(mine), "sees_all": sees_all },
        "total": total, "offset": offset, "people": page, "groups": groups,
        "idp_error": dir.error,
    })).into_response()
}

#[derive(serde::Deserialize, Default)]
pub struct AuditQuery {
    /// One org's handle: its members, its projects and who reaches them.
    #[serde(default)]
    org: Option<String>,
}

/// `GET /security/people` — everyone, for an audit; `?org=<handle>` for one org.
///
/// The whole platform is SuperAdmin's and SecurityAdmin's. One org is also its
/// own admins' and security admins': its members, its projects and everyone who
/// reaches them, but not anyone's platform roles or other orgs (those are the
/// platform's business, not the org's).
pub async fn security_people_handler(
    Extension(idp): Extension<crate::IdpAdminState>,
    principal: Option<Extension<Principal>>,
    Query(q): Query<AuditQuery>,
) -> Response {
    let Some((me, my_roles)) = human(principal) else {
        return refuse(StatusCode::UNAUTHORIZED, "Sign in first.");
    };
    let is_infosec = infosec(&my_roles);
    let Some(snap) = access::current_snapshot().await else {
        return refuse(StatusCode::NOT_IMPLEMENTED, "Project access is known when it is enforced (FlowR EE).");
    };
    let org = q.org.as_deref().map(str::trim).filter(|o| !o.is_empty()).map(String::from);
    match &org {
        None if !is_infosec => return refuse(StatusCode::FORBIDDEN, "SuperAdmin or SecurityAdmin role required"),
        Some(h) if !snap.org_handles().contains(h) || (!is_infosec && !snap.security_orgs_of(&me).contains(h)) => {
            return refuse(StatusCode::NOT_FOUND, &format!("No org {h:?} whose security you look after."));
        }
        _ => {}
    }
    // Platform roles are read only for the whole-platform view.
    let dir = directory(&idp, org.is_none()).await;
    let namespaces = match &org { Some(h) => snap.org_namespaces(h), None => snap.namespaces() };
    // person → namespace → (best access, capacities)
    let mut reach: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut projects: Vec<Value> = Vec::new();
    for ns in &namespaces {
        let r = routes_with_audit(&snap, ns, &dir);
        let mut named: Vec<Route> = r.people.iter().filter(|p| p.capacity != SECURITY_ADMIN_CAPACITY).cloned().collect();
        // Another org reaching it through exposure: its members, by name.
        for g in &r.groups {
            if let Some(h) = g.group.strip_prefix("org:") {
                for u in snap.members_of(h) {
                    named.push(Route { user: u, capacity: g.capacity, access: g.access });
                }
            }
        }
        for (u, (best, caps)) in by_person(&named) {
            reach.entry(u).or_default().push(json!({ "namespace": ns, "access": best, "capacities": caps }));
        }
        projects.push(json!({ "namespace": ns, "kind": r.kind, "owner": r.owner, "private": r.private,
            "everyone": r.groups.iter().any(|g| g.group == "everyone"), "people": named.iter().map(|p| &p.user).collect::<BTreeSet<_>>().len() }));
    }
    let ids: BTreeSet<String> = match &org {
        // The org's members and whoever reaches its projects.
        Some(h) => snap.members_of(h).into_iter().chain(reach.keys().cloned()).collect(),
        None => dir.people.keys().cloned().chain(snap.known_users()).collect(),
    };
    let users: Vec<Value> = ids.iter().map(|u| {
        let p = person(&dir, &snap, u);
        let mut roles: Vec<&String> = if org.is_none() { dir.roles.get(u).map(|r| r.iter().collect()).unwrap_or_default() } else { Vec::new() };
        roles.sort_by_key(|r| roles::ALL.iter().position(|x| x == r).unwrap_or(99));
        let orgs: Vec<Value> = snap.orgs_of(u).into_iter()
            .filter(|(h, _)| org.as_ref().is_none_or(|o| o == h))
            .map(|(org, role)| json!({ "org": org, "role": role })).collect();
        json!({
            "id": u, "email": p.email, "name": p.name, "enabled": p.enabled, "account": dir.people.contains_key(u),
            "roles": roles,
            "realm_roles": if org.is_none() { dir.realm.get(u).map(|r| r.iter().cloned().collect::<Vec<_>>()).unwrap_or_default() } else { Vec::new() },
            "orgs": orgs,
            "projects": reach.remove(u).unwrap_or_default(),
        })
    }).collect();
    let orgs = match &org { Some(h) => vec![h.clone()], None => snap.org_handles() };
    Json(json!({ "count": users.len(), "org": org, "users": users, "projects": projects, "orgs": orgs, "idp_error": dir.error })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_row_per_person_with_their_best_access() {
        let r = vec![
            Route { user: "bob".into(), capacity: "viewer", access: "read" },
            Route { user: "bob".into(), capacity: "org-admin", access: "write" },
            Route { user: "sec".into(), capacity: SECURITY_ADMIN_CAPACITY, access: "audit" },
        ];
        let m = by_person(&r);
        assert_eq!(m["bob"], ("write".to_string(), vec!["org-admin", "viewer"]));
        assert_eq!(m["sec"].0, "audit");
    }
}
