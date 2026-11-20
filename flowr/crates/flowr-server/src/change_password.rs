//! Change your own password (`POST /auth/change-password {current_password,
//! new_password}`), signed in.
//!
//! The server proves the current password itself, then sets the new one where the
//! account lives: an A3T local account (Argon2 in the platform `users` table), or a
//! Keycloak account (the realm's password policy applies). It always acts on the
//! caller's own account; nobody changes someone else's password here, and a
//! SuperAdmin acting as someone cannot change theirs.

use axum::{extract::Extension, http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::Deserialize;
use serde_json::{json, Map};
use std::sync::Arc;
use tracing::{info, warn};

use crate::auth::{hash_password, verify_password, AuthConfig, Claims};
use crate::auth_routes::{keycloak_login, KcLogin};
use crate::first_password::refuse;

#[derive(Deserialize)]
pub struct ChangePasswordBody {
    pub current_password: String,
    /// ID-9: the authenticator code, for accounts whose sign-in needs one.
    #[serde(default)]
    pub otp: Option<String>,
    pub new_password: String,
}

pub async fn change_password_handler(
    Extension(config): Extension<Arc<AuthConfig>>,
    Extension(idp): Extension<crate::IdpAdminState>,
    principal: Option<Extension<flowr_policy::Principal>>,
    claims: Option<Extension<Claims>>,
    Json(body): Json<ChangePasswordBody>,
) -> Response {
    let Some(Extension(flowr_policy::Principal::Human { user_id, impersonated_by, .. })) = principal else {
        return refuse(StatusCode::UNAUTHORIZED, "Sign in to change your password.");
    };
    if impersonated_by.is_some() {
        return refuse(StatusCode::FORBIDDEN, "Stop acting as someone before changing a password.");
    }
    let email = claims.map(|c| c.0.email.clone()).unwrap_or_default();
    if body.current_password.is_empty() || body.new_password.is_empty() {
        return refuse(StatusCode::BAD_REQUEST, "Your current password and a new password are required.");
    }
    if body.new_password.chars().count() < 8 {
        return refuse(StatusCode::BAD_REQUEST, "Your new password needs at least 8 characters.");
    }
    if body.new_password == body.current_password {
        return refuse(StatusCode::BAD_REQUEST, "Choose a password different from the current one.");
    }

    // An A3T local account (the bootstrap superadmin, CE sign-ups).
    let platform = config.platform.get().await;
    let filter = format!("id = '{}'", user_id.replace('\'', "''"));
    let local_hash = match platform.query("users", Some(&filter), 1).await {
        Ok(batches) => batches.into_iter().find(|b| b.num_rows() > 0).and_then(|b| {
            b.column_by_name("password_hash")
                .and_then(|c| c.as_any().downcast_ref::<arrow_array::StringArray>())
                .map(|a| a.value(0).to_string())
        }),
        Err(e) => {
            warn!(error = %e, "change-password: users table unreadable");
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "Your password couldn't be changed. Try again in a moment.");
        }
    };
    if let Some(stored) = local_hash {
        if !verify_password(&body.current_password, &stored) {
            return refuse(StatusCode::UNAUTHORIZED, "Your current password is not right.");
        }
        let mut values = Map::new();
        values.insert("password_hash".into(), json!(hash_password(&body.new_password)));
        values.insert("updated_at".into(), json!(chrono::Utc::now().to_rfc3339()));
        if let Err(e) = platform.update("users", &filter, &values).await {
            warn!(error = %e, "change-password: update failed");
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "Your password couldn't be changed. Try again in a moment.");
        }
        info!(user = %user_id, "change-password: local password changed");
        return Json(json!({ "ok": true })).into_response();
    }

    // A Keycloak account: Keycloak proves the current password.
    if email.is_empty() {
        return refuse(StatusCode::BAD_REQUEST, "This account has no email to sign in with.");
    }
    match keycloak_login(&config, &email, &body.current_password, body.otp.as_deref()).await {
        KcLogin::Ok(_) | KcLogin::SetupRequired => {}
        KcLogin::Failed => return refuse(StatusCode::UNAUTHORIZED, "Your current password is not right."),
    }
    #[cfg(feature = "ee")]
    {
        if let Err(resp) = crate::first_password::set_keycloak_password(&idp, &email, &body.new_password).await {
            return resp;
        }
        info!(user = %user_id, "change-password: Keycloak password changed");
        Json(json!({ "ok": true })).into_response()
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = idp;
        refuse(StatusCode::SERVICE_UNAVAILABLE, "Changing this password isn't available in this edition.")
    }
}
