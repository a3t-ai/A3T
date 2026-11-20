//! The canonical A3T role vocabulary — the single source of truth for role
//! names, shared by CE, EE, and every Cedar policy (analogous to the `actions`
//! registry). Lives in the CE contract crate because *roles are infrastructure*:
//! CE defines the vocabulary; EE's AuthN layer (JWT today, OIDC/SSO later) maps a
//! provider's role claim onto these names, and the policies check them.
//!
//! The platform issues a single lowercase role per user (`superadmin`, `admin`,
//! `operator`, `viewer`); [`canonicalize`] maps that onto the canonical role(s).

/// Full control, including InfoSec/policy authoring.
pub const SUPER_ADMIN: &str = "SuperAdmin";
/// InfoSec: owns Cedar policy resources (the SoD seam, §6.4). Granted by
/// `superadmin` today; a distinct assignable role once roles are first-class.
pub const SECURITY_ADMIN: &str = "SecurityAdmin";
/// Platform administration (resources, settings, users) short of policy.
pub const ADMIN: &str = "Admin";
/// Runs flows/agents; no administrative mutation.
pub const OPERATOR: &str = "Operator";
/// Read-only.
pub const VIEWER: &str = "Viewer";

/// Every canonical role. Policies and the (future) role-seeding reference this.
pub const ALL: &[&str] = &[SUPER_ADMIN, SECURITY_ADMIN, ADMIN, OPERATOR, VIEWER];

/// The role table (ID-1), `a3t/policies/roles.yaml`, compiled into each release.
pub const ROLES_YAML: &str = include_str!("../../../a3t/policies/roles.yaml");

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoleTable {
    pub version: u32,
    pub roles: Vec<Role>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Role {
    pub id: String,
    /// `cedar`, `platform`, `org` or `project`.
    pub scope: String,
    pub label: String,
    pub what: String,
    /// The Cedar roles it gives where it applies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carries: Vec<String>,
    /// In a project: `write`, `run` or `read`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<u8>,
}

/// The table as shipped (it is checked by a test, so it always parses).
pub fn table() -> &'static RoleTable {
    static T: std::sync::OnceLock<RoleTable> = std::sync::OnceLock::new();
    T.get_or_init(|| serde_yaml::from_str(ROLES_YAML).expect("a3t/policies/roles.yaml parses"))
}

/// A role of `scope` by id.
pub fn role(scope: &str, id: &str) -> Option<&'static Role> {
    table().roles.iter().find(|r| r.scope == scope && r.id == id)
}

/// Map a provider's `role` claim (a platform role in the sign-in realm) to the
/// Cedar roles it carries, from the role table: the hierarchy is expanded
/// there, so a higher role holds every lower one and every policy stays a flat
/// membership check (`principal.roles.contains("Admin")` matches a superadmin
/// too). Unknown roles pass through unchanged — never silently drop a role.
pub fn canonicalize(provider_role: &str) -> Vec<String> {
    let id = provider_role.trim().to_ascii_lowercase().replace(['_', '-'], "");
    match role("platform", &id) {
        Some(r) => r.carries.clone(),
        None => vec![provider_role.to_string()],
    }
}

/// What a project role allows in its project: `write`, `run` or `read`.
pub fn project_access(role_id: &str) -> Option<&'static str> {
    role("project", role_id).and_then(|r| r.access.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(role: &str, s: &str) -> bool {
        canonicalize(role).contains(&s.to_string())
    }

    #[test]
    fn superadmin_grants_infosec_and_all_lower_roles() {
        for r in [SUPER_ADMIN, SECURITY_ADMIN, ADMIN, OPERATOR, VIEWER] {
            assert!(has("superadmin", r), "superadmin should grant {r}");
        }
    }

    #[test]
    fn hierarchy_expands_downward_only() {
        // admin holds admin/operator/viewer but NOT superadmin/securityadmin.
        assert!(has("admin", ADMIN) && has("admin", OPERATOR) && has("admin", VIEWER));
        assert!(!has("admin", SUPER_ADMIN) && !has("admin", SECURITY_ADMIN));
        // viewer holds only viewer.
        assert_eq!(canonicalize("viewer"), vec![VIEWER]);
    }

    #[test]
    fn the_table_parses_and_is_one_vocabulary() {
        let t = table();
        for r in &t.roles {
            assert!(["cedar", "platform", "org", "project"].contains(&r.scope.as_str()), "{}: scope {}", r.id, r.scope);
            for c in &r.carries {
                assert!(ALL.contains(&c.as_str()), "{} carries {c}, which is not a Cedar role", r.id);
            }
        }
        for c in ALL {
            assert!(role("cedar", c).is_some(), "the Cedar role {c} is in the table");
        }
        // "developer" is a project role, not a sign-in role.
        assert!(role("platform", "developer").is_none());
        assert_eq!(project_access("developer"), Some("write"));
        assert_eq!(canonicalize("developer"), vec!["developer".to_string()]);
    }

    #[test]
    fn unknown_role_passes_through() {
        assert_eq!(canonicalize("custom-role"), vec!["custom-role".to_string()]);
    }
}
