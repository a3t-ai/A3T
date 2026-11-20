//! `POST /auth/refresh {refresh_token}` — a new session for a signed-in person.
//!
//! The refresh token says only who (`sub`). Everything else is read again now, so
//! a role taken away or an account disabled since sign-in takes effect at the next
//! refresh rather than lasting the refresh token's whole life:
//! - an A3T local account: its row in the platform `users` table (must be active);
//! - a Keycloak account: the user and its realm roles from the admin API (must be
//!   enabled).
//! Impersonation sessions get no refresh token (impersonate.rs), so none comes here.

use axum::{extract::Extension, http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::Deserialize;
use std::sync::Arc;
use tracing::{info, warn};

use crate::auth::AuthConfig;
use crate::auth_routes::session_tokens;
use crate::sessions::{self, Turn};
use crate::first_password::refuse;

#[derive(Deserialize)]
pub struct RefreshBody {
    pub refresh_token: String,
}

pub async fn refresh_handler(
    Extension(config): Extension<Arc<AuthConfig>>,
    Extension(idp): Extension<crate::IdpAdminState>,
    Json(body): Json<RefreshBody>,
) -> Response {
    let expired = || refuse(StatusCode::UNAUTHORIZED, "Your session has ended. Sign in again.");
    let Ok(rt) = crate::auth::validate_refresh(&body.refresh_token, &config, false) else {
        return expired();
    };
    let sub = rt.sub.clone();

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
                info!(user = %sub, "refresh: local account still active");
                return renew(&config, &rt, (sub.clone(), col("org_id"), col("role"), col("email"), col("name"))).await;
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
        return renew(&config, &rt, (sub, "org-default".to_string(), role, email.clone(), if name.is_empty() { email } else { name })).await;
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = idp;
        expired()
    }
}

/// The person still holds the account: swap the refresh token and issue new tokens.
async fn renew(config: &AuthConfig, rt: &crate::auth::RefreshClaims, identity: (String, String, String, String, String)) -> Response {
    match sessions::rotate(config, &rt.sid, &rt.sub, &rt.jti).await {
        Ok(Turn::Rotated(jti, ends)) => session_tokens(config, identity, &rt.sid, &jti, ends),
        // Another tab (or a retried request) renewed it a moment ago: the newer
        // tokens are in the browser's storage. Nothing is revoked.
        Ok(Turn::Stale) => (StatusCode::CONFLICT, axum::Json(serde_json::json!({
            "error": "This sign-in was renewed a moment ago. Use the newer token.", "code": "refresh_stale",
        }))).into_response(),
        Ok(Turn::Refused(why)) => {
            info!(user = %rt.sub, sid = %rt.sid, why, "refresh refused");
            refuse(StatusCode::UNAUTHORIZED, "Your session has ended. Sign in again.")
        }
        Err(e) => {
            warn!(error = %e, "refresh: sessions table unavailable");
            refuse(StatusCode::SERVICE_UNAVAILABLE, "Try again in a moment.")
        }
    }
}
