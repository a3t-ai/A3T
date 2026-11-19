//! A SuperAdmin acting as a user, or as a role (`POST /auth/impersonate`).
//!
//! To see what someone sees — and what they are refused — a SuperAdmin takes a
//! short-lived token **for that user** (their id, role and home org; every
//! decision is made as them), or for a role (a fresh user with only that role
//! and no memberships). The token names the SuperAdmin in its `act` claim (RFC
//! 8693), so the decision log records both on every call; starting is itself
//! logged. It lasts at most an hour and cannot be refreshed or used to act as
//! anyone else: going back is the console restoring the SuperAdmin's own session.

use std::sync::Arc;

use axum::{extract::Extension, http::StatusCode, response::{IntoResponse, Response}, Json};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::auth::{Actor, AuthConfig, Claims};

const MAX_SECS: u64 = 3600;
const ROLES: [&str; 4] = ["securityadmin", "admin", "operator", "viewer"];

#[derive(Deserialize)]
pub struct ImpersonateBody {
    /// Act as this user (Keycloak user id, or a local A3T user id).
    pub user_id: Option<String>,
    /// Or as a role: securityadmin, admin, operator or viewer.
    pub role: Option<String>,
}

fn refuse(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// Who the token will be for: (sub, email, name, role, org).
struct Target { sub: String, email: String, name: String, role: String, org: String }

pub async fn impersonate_handler(
    Extension(config): Extension<Arc<AuthConfig>>,
    Extension(idp): Extension<crate::IdpAdminState>,
    principal: Option<Extension<flowr_policy::Principal>>,
    claims: Option<Extension<Claims>>,
    Json(body): Json<ImpersonateBody>,
) -> Response {
    use flowr_policy::{roles, Principal};
    let Some(Extension(me)) = principal else {
        return refuse(StatusCode::UNAUTHORIZED, "Sign in first.");
    };
    let Principal::Human { user_id: my_id, roles: my_roles, impersonated_by, .. } = &me else {
        return refuse(StatusCode::FORBIDDEN, "Only a person can act as someone else.");
    };
    if impersonated_by.is_some() {
        return refuse(StatusCode::FORBIDDEN, "Stop acting as someone else first.");
    }
    if !my_roles.iter().any(|r| r == roles::SUPER_ADMIN) {
        return refuse(StatusCode::FORBIDDEN, "Only a SuperAdmin can act as another user or role.");
    }
    let my_email = claims.map(|c| c.0.email.clone()).unwrap_or_default();

    let target = match (body.user_id.as_deref().map(str::trim).filter(|s| !s.is_empty()), body.role.as_deref().map(str::trim).filter(|s| !s.is_empty())) {
        (Some(_), Some(_)) | (None, None) => return refuse(StatusCode::BAD_REQUEST, "Choose a user or a role."),
        (None, Some(role)) => {
            let role = role.to_ascii_lowercase();
            if !ROLES.contains(&role.as_str()) {
                return refuse(StatusCode::BAD_REQUEST, "The role is securityadmin, admin, operator or viewer.");
            }
            Target {
                // A user of its own per SuperAdmin and role: no memberships, so it
                // sees what anyone new with that role sees.
                sub: format!("as-{role}:{my_id}"),
                email: String::new(),
                name: format!("{} (role view)", role),
                role,
                org: "org-default".into(),
            }
        }
        (Some(user), None) => match resolve_user(&config, &idp, user).await {
            Ok(t) => t,
            Err((status, msg)) => return refuse(status, &msg),
        },
    };
    if target.sub == *my_id {
        return refuse(StatusCode::BAD_REQUEST, "That is you.");
    }

    let now = chrono::Utc::now().timestamp() as usize;
    let secs = config.token_expiry_secs.min(MAX_SECS);
    let token_claims = Claims {
        sub: target.sub.clone(), org: target.org.clone(), role: target.role.clone(),
        email: target.email.clone(), name: target.name.clone(), aud: config.resource_uri.clone(),
        exp: now + secs as usize, iat: now,
        act: Some(Actor { sub: my_id.clone(), email: my_email.clone() }),
    };
    let token = match encode(&Header::default(), &token_claims, &EncodingKey::from_secret(config.jwt_secret.as_bytes())) {
        Ok(t) => t,
        Err(e) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, &format!("token: {e}")),
    };

    // Starting is a decision of its own in the log, by the SuperAdmin.
    if let Ok(action) = flowr_policy::Action::new("auth:impersonate") {
        let mut resource = flowr_policy::Resource::new("User", target.sub.clone());
        resource.attributes.insert("role".into(), Value::String(target.role.clone()));
        resource.attributes.insert("email".into(), Value::String(target.email.clone()));
        let req = flowr_policy::ExecutionRequest {
            principal: me.clone(), action, resource,
            context: Default::default(), attributes: Default::default(), params: Value::Null,
        };
        flowr_policy::audit::record(&req, &flowr_policy::PolicyDecision::Allow);
    }
    info!(superadmin = %my_id, superadmin_email = %my_email, as_user = %target.sub, as_email = %target.email,
          as_role = %target.role, minutes = secs / 60, "impersonation started");

    Json(json!({
        "access_token": token,
        "token_type": "Bearer",
        "expires_in": secs,
        "user": { "id": target.sub, "email": target.email, "name": target.name, "role": target.role, "org_id": target.org },
        "act": { "sub": my_id, "email": my_email },
    })).into_response()
}

/// A user's identity, A3T role and home org: from Keycloak (SSO and realm
/// users), else the local A3T users table.
async fn resolve_user(config: &AuthConfig, idp: &crate::IdpAdminState, user: &str) -> Result<Target, (StatusCode, String)> {
    let safe = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.@:".contains(c));
    if !safe(user) || user.len() > 128 {
        return Err((StatusCode::BAD_REQUEST, "That is not a user id.".into()));
    }
    let db = config.platform.get().await;
    let rows = |b: Vec<arrow_array::RecordBatch>| flowr_ce::handlers::call_lancedb::batches_to_json(&b);
    let s = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();

    let mut found: Option<(String, String, String)> = None; // email, name, role
    #[cfg(feature = "ee")]
    if let Some(cfg) = a3t_security::oidc_admin::IdpAdminConfig::from_issuer(&idp.issuer, idp.admin_user.clone(), idp.admin_password.clone()) {
        use a3t_security::oidc_admin as kc;
        match kc::admin_token(&idp.client, &cfg).await {
            Ok(tok) => {
                if let Ok(u) = kc::get_user(&idp.client, &cfg, &tok, user).await {
                    let realm_roles = kc::user_realm_roles(&idp.client, &cfg, &tok, user).await.unwrap_or_default();
                    let name = [s(&u, "firstName"), s(&u, "lastName")].join(" ").trim().to_string();
                    found = Some((s(&u, "email"), if name.is_empty() { s(&u, "username") } else { name }, crate::auth_routes::pick_role(&realm_roles)));
                }
            }
            Err(e) => warn!(error = %e, "impersonate: Keycloak admin unavailable; trying local users"),
        }
    }
    #[cfg(not(feature = "ee"))]
    let _ = idp;
    if found.is_none() {
        if let Ok(b) = db.query("users", Some(&format!("id = '{user}'")), 1).await {
            if let Some(u) = rows(b).first() {
                found = Some((s(u, "email"), s(u, "name"), s(u, "role")));
            }
        }
    }
    let Some((email, name, role)) = found else {
        return Err((StatusCode::NOT_FOUND, "No such user.".into()));
    };
    // Home org: the one their sign-in placed them in, else their first membership.
    let memberships = db.query("org_members", Some(&format!("user_id = \"{user}\"")), 100).await.map(rows).unwrap_or_default();
    let org = memberships.iter().find(|m| s(m, "added_by") == "sso").or(memberships.first())
        .map(|m| s(m, "org_id")).filter(|o| !o.is_empty()).unwrap_or_else(|| "org-default".into());
    Ok(Target { sub: user.to_string(), email, name, role, org })
}
