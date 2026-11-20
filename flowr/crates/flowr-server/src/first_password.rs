//! First sign-in with a temporary password (`POST /auth/first-password
//! {email, password, new_password}`).
//!
//! An admin can make an account whose first password is temporary (Security ›
//! Identity › Users, "They change the password at first sign-in"). Keycloak then
//! refuses the console's password grant with "Account is not fully set up" — but
//! only once the password is right, so that answer proves the temporary password.
//! `/auth/login` turns it into `403 {code: "password_change_required"}`; the
//! console asks for a new password and posts it here. Here the temporary password
//! is proved the same way again, the new one is set (no longer temporary, the
//! required action cleared), and the person is signed in with it.
//!
//! Nothing here works without the temporary password: a wrong one is the same
//! "Invalid credentials" as at /auth/login, and an account with no pending change
//! is told to sign in normally.

use axum::{extract::Extension, http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

use crate::auth::AuthConfig;
use crate::auth_routes::{issue_session, keycloak_login, KcLogin};

#[derive(Deserialize)]
pub struct FirstPasswordBody {
    pub email: String,
    /// The temporary password the admin gave.
    pub password: String,
    pub new_password: String,
}

pub(crate) fn refuse(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

pub async fn first_password_handler(
    Extension(config): Extension<Arc<AuthConfig>>,
    Extension(idp): Extension<crate::IdpAdminState>,
    Json(body): Json<FirstPasswordBody>,
) -> Response {
    let email = body.email.trim().to_ascii_lowercase();
    if email.is_empty() || body.password.is_empty() || body.new_password.is_empty() {
        return refuse(StatusCode::BAD_REQUEST, "Email, temporary password and new password are required.");
    }
    if body.new_password.chars().count() < 8 {
        return refuse(StatusCode::BAD_REQUEST, "Your new password needs at least 8 characters.");
    }
    if body.new_password == body.password {
        return refuse(StatusCode::BAD_REQUEST, "Choose a password different from the temporary one.");
    }
    // Prove the temporary password: Keycloak says "not fully set up" only when it is right.
    match keycloak_login(&config, &email, &body.password).await {
        KcLogin::SetupRequired => {}
        KcLogin::Ok(_) => return refuse(StatusCode::CONFLICT, "Your account is already set up. Sign in with your password."),
        KcLogin::Failed => return refuse(StatusCode::UNAUTHORIZED, "Invalid credentials"),
    }
    #[cfg(feature = "ee")]
    {
        if let Err(resp) = set_keycloak_password(&idp, &email, &body.new_password).await {
            return resp;
        }
        info!("first-password: new password set");
        return match keycloak_login(&config, &email, &body.new_password).await {
            KcLogin::Ok(identity) => issue_session(&config, identity).await,
            KcLogin::SetupRequired => refuse(StatusCode::FORBIDDEN, "Your password is set, but your account needs more setup. Ask your administrator."),
            KcLogin::Failed => refuse(StatusCode::SERVICE_UNAVAILABLE, "Your password is set. Sign in with it."),
        };
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = idp;
        refuse(StatusCode::SERVICE_UNAVAILABLE, "Setting a password isn't available in this edition. Ask your administrator.")
    }
}

/// Set a Keycloak account's password (not temporary) and clear a pending
/// "update password" action. Keycloak checks it against the realm's password
/// policy. The caller has already proved the person may do this.
#[cfg(feature = "ee")]
pub(crate) async fn set_keycloak_password(idp: &crate::IdpAdminState, email: &str, new_password: &str) -> Result<(), Response> {
    use a3t_security::oidc_admin as kc;
    let Some(cfg) = kc::IdpAdminConfig::from_issuer(&idp.issuer, idp.admin_user.clone(), idp.admin_password.clone()) else {
        return Err(refuse(StatusCode::SERVICE_UNAVAILABLE, "Setting a password isn't available here. Ask your administrator."));
    };
    let token = kc::admin_token(&idp.client, &cfg).await.map_err(|e| {
        warn!(error = %e, "set password: Keycloak admin unavailable");
        refuse(StatusCode::SERVICE_UNAVAILABLE, "The sign-in service can't be reached. Try again in a moment.")
    })?;
    let admin = format!("{}/admin/realms/{}", cfg.base_url, cfg.realm);
    let mut find = reqwest::Url::parse(&format!("{admin}/users")).expect("admin url");
    find.query_pairs_mut().append_pair("username", email).append_pair("exact", "true");
    let users: Vec<serde_json::Value> = match idp.client.get(find).bearer_auth(&token).send().await {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
        _ => Vec::new(),
    };
    let Some(id) = users.first().and_then(|u| u.get("id")).and_then(|v| v.as_str()).map(String::from) else {
        // Keycloak just accepted this username; not finding it is a fault, not a refusal.
        warn!("set password: the account Keycloak accepted was not found by username");
        return Err(refuse(StatusCode::SERVICE_UNAVAILABLE, "Your password couldn't be set. Try again, or ask your administrator."));
    };
    // The new password, no longer temporary. Keycloak checks it against the realm's password policy.
    let set = idp.client.put(format!("{admin}/users/{id}/reset-password")).bearer_auth(&token)
        .json(&json!({ "type": "password", "value": new_password, "temporary": false }))
        .send().await;
    match set {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => {
            let status = r.status();
            let why: serde_json::Value = r.json().await.unwrap_or_default();
            let msg = why.get("error_description").or_else(|| why.get("errorMessage")).and_then(|m| m.as_str())
                .unwrap_or("That password isn't allowed. Try a longer one.").to_string();
            info!(user = %id, %status, "set password: new password refused");
            return Err(refuse(StatusCode::BAD_REQUEST, &msg));
        }
        Err(e) => {
            warn!(user = %id, error = %e, "set password: Keycloak unreachable");
            return Err(refuse(StatusCode::SERVICE_UNAVAILABLE, "The sign-in service can't be reached. Try again in a moment."));
        }
    }
    // Clear the pending "update password" action, keeping any other.
    if let Ok(mut rep) = kc::get_user(&idp.client, &cfg, &token, &id).await {
        let pending: Vec<serde_json::Value> = rep.get("requiredActions").and_then(|a| a.as_array()).cloned().unwrap_or_default();
        if pending.iter().any(|a| a == "UPDATE_PASSWORD") {
            let rest: Vec<serde_json::Value> = pending.into_iter().filter(|a| a != "UPDATE_PASSWORD").collect();
            rep["requiredActions"] = json!(rest);
            if let Err(e) = idp.client.put(format!("{admin}/users/{id}")).bearer_auth(&token).json(&rep).send().await {
                warn!(user = %id, error = %e, "set password: could not clear the pending action");
            }
        }
    }
    Ok(())
}
