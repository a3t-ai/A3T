//! `POST /auth/refresh {refresh_token}` — a new session for a signed-in person.
//!
//! The refresh token says only who (`sub`). Everything else is read again now, so
//! a role taken away or an account disabled since sign-in takes effect at the next
//! refresh rather than lasting the refresh token's whole life:
//! - an A3T local account: its row in the platform `users` table (must be active);
//! - a Keycloak account: the user and its realm roles from the admin API (must be
//!   enabled).
//! Impersonation sessions get no refresh token (impersonate.rs), so none comes here.

use axum::{extract::Extension, http::StatusCode, response::Response, Json};
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::Deserialize;
use std::sync::Arc;
use tracing::{info, warn};

use crate::auth::AuthConfig;
use crate::auth_routes::issue_session;
use crate::first_password::refuse;

#[derive(Deserialize)]
pub struct RefreshBody {
    pub refresh_token: String,
}

#[derive(Deserialize)]
struct RefreshClaims {
    sub: String,
    #[serde(default)]
    refresh: bool,
}

pub async fn refresh_handler(
    Extension(config): Extension<Arc<AuthConfig>>,
    Extension(idp): Extension<crate::IdpAdminState>,
    Json(body): Json<RefreshBody>,
) -> Response {
    let expired = || refuse(StatusCode::UNAUTHORIZED, "Your session has ended. Sign in again.");
    let mut validation = Validation::default();
    validation.set_required_spec_claims(&["exp", "sub"]);
    validation.validate_aud = false;
    let Ok(data) = decode::<RefreshClaims>(&body.refresh_token, &DecodingKey::from_secret(config.jwt_secret.as_bytes()), &validation) else {
        return expired();
    };
    // An access token is not a refresh token.
    if !data.claims.refresh || data.claims.sub.is_empty() {
        return expired();
    }
    let sub = data.claims.sub;

    // An A3T local account.
    let platform = config.platform.get().await;
    let filter = format!("id = '{}'", sub.replace('\'', "''"));
    match platform.query("users", Some(&filter), 1).await {
        Ok(batches) => {
            if let Some(b) = batches.into_iter().find(|b| b.num_rows() > 0) {
                let col = |name: &str| b.column_by_name(name)
                    .and_then(|c| c.as_any().downcast_ref::<arrow_array::StringArray>())
                    .map(|a| a.value(0).to_string())
                    .unwrap_or_default();
                if col("status") != "active" {
                    return expired();
                }
                info!(user = %sub, "refresh: local session renewed");
                return issue_session(&config, (sub.clone(), col("org_id"), col("role"), col("email"), col("name")));
            }
        }
        Err(e) => {
            warn!(error = %e, "refresh: users table unreadable");
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "Try again in a moment.");
        }
    }

    // A Keycloak account, read again through the admin API.
    #[cfg(feature = "ee")]
    {
        use a3t_security::oidc_admin as kc;
        let Some(cfg) = kc::IdpAdminConfig::from_issuer(&idp.issuer, idp.admin_user.clone(), idp.admin_password.clone()) else {
            return expired();
        };
        let token = match kc::admin_token(&idp.client, &cfg).await {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "refresh: Keycloak admin unavailable");
                return refuse(StatusCode::SERVICE_UNAVAILABLE, "Try again in a moment.");
            }
        };
        let Ok(user) = kc::get_user(&idp.client, &cfg, &token, &sub).await else {
            return expired();
        };
        if user.get("enabled").and_then(|v| v.as_bool()) != Some(true) {
            return expired();
        }
        let roles = match kc::user_realm_roles(&idp.client, &cfg, &token, &sub).await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "refresh: realm roles unreadable");
                return refuse(StatusCode::SERVICE_UNAVAILABLE, "Try again in a moment.");
            }
        };
        let s = |k: &str| user.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let email = if s("email").is_empty() { s("username") } else { s("email") };
        let name = format!("{} {}", s("firstName"), s("lastName")).trim().to_string();
        let role = crate::auth_routes::pick_role(&roles);
        info!(user = %sub, role = %role, "refresh: Keycloak session renewed");
        return issue_session(&config, (sub, "org-default".to_string(), role, email.clone(), if name.is_empty() { email } else { name }));
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = idp;
        expired()
    }
}
