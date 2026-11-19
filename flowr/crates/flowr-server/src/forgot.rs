//! "Forgot password" (`POST /auth/forgot {email, return}`).
//!
//! Keycloak does the work: FlowR finds the realm user by email and asks Keycloak
//! to email them a link to set a new password (execute-actions-email
//! UPDATE_PASSWORD, valid 30 minutes); the link ends back on the console's sign-in
//! page. The answer is the same whether or not the account exists, so the form
//! can't be used to find out who has one. Not sent:
//! - to someone who signs in through their organisation's own provider (their
//!   password is theirs to reset there);
//! - more than once a minute for the same address.
//! If the realm has no mail server, the form says so (that reveals nothing about
//! any account).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{extract::Extension, http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::auth::AuthConfig;

const LINK_SECS: u64 = 1800;
const PER_ADDRESS: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
pub struct ForgotBody {
    pub email: String,
    #[serde(rename = "return")]
    pub ret: Option<String>,
}

fn answer() -> Response {
    Json(json!({ "ok": true, "message": "If that address has an account, a link to set a new password is on its way." })).into_response()
}

fn recently_asked(email: &str) -> bool {
    static SEEN: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let mut seen = SEEN.get_or_init(Default::default).lock().unwrap();
    let now = Instant::now();
    seen.retain(|_, t| now.duration_since(*t) < PER_ADDRESS);
    if seen.contains_key(email) {
        return true;
    }
    seen.insert(email.to_string(), now);
    false
}

pub async fn forgot_handler(
    Extension(config): Extension<Arc<AuthConfig>>,
    Extension(idp): Extension<crate::IdpAdminState>,
    Json(body): Json<ForgotBody>,
) -> Response {
    let email = body.email.trim().to_ascii_lowercase();
    if !email.contains('@') || email.len() > 254 || email.contains(['"', '\\', '&', '?', '#', '/']) {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Enter the email you sign in with." }))).into_response();
    }
    #[cfg(feature = "ee")]
    {
        use a3t_security::oidc_admin as kc;
        let Some(cfg) = kc::IdpAdminConfig::from_issuer(&idp.issuer, idp.admin_user.clone(), idp.admin_password.clone()) else {
            return unavailable("Password reset isn't available here. Ask your administrator.");
        };
        let token = match kc::admin_token(&idp.client, &cfg).await {
            Ok(t) => t,
            Err(e) => { warn!(error = %e, "forgot: Keycloak admin unavailable"); return unavailable("Password reset is unavailable right now. Try again later."); }
        };
        let admin = format!("{}/admin/realms/{}", cfg.base_url, cfg.realm);
        let get = |url: String| { let c = idp.client.clone(); let t = token.clone(); async move {
            c.get(url).bearer_auth(t).send().await.ok()?.json::<Value>().await.ok()
        } };
        // No mail server in the realm: say so — true for every address alike.
        let smtp_ready = get(admin.clone()).await
            .and_then(|r| r.get("smtpServer").and_then(|s| s.get("host")).and_then(|h| h.as_str()).map(|h| !h.is_empty()))
            .unwrap_or(false);
        if !smtp_ready {
            warn!("forgot: the realm has no mail server (smtpServer.host); no reset email can be sent");
            return unavailable("Password reset by email isn't set up yet. Ask your administrator to reset it.");
        }
        if recently_asked(&email) {
            return answer();
        }
        let users = get(format!("{admin}/users?email={email}&exact=true")).await.and_then(|v| v.as_array().cloned()).unwrap_or_default();
        let Some(user) = users.into_iter().find(|u| u.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false)) else {
            info!("forgot: no enabled account for that address (answered the same)");
            return answer();
        };
        let id = user.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let federated = get(format!("{admin}/users/{id}/federated-identity")).await
            .and_then(|v| v.as_array().map(|a| !a.is_empty())).unwrap_or(false);
        if federated {
            info!(user = %id, "forgot: signs in through their organisation; not sent");
            return answer();
        }
        let back = format!("{}/login", crate::sso::return_origin(&config, body.ret.as_deref()));
        let mut url = reqwest::Url::parse(&format!("{admin}/users/{id}/execute-actions-email")).expect("admin url");
        url.query_pairs_mut()
            .append_pair("client_id", &config.oidc_client_id)
            .append_pair("redirect_uri", &back)
            .append_pair("lifespan", &LINK_SECS.to_string());
        match idp.client.put(url).bearer_auth(&token).json(&json!(["UPDATE_PASSWORD"])).send().await {
            Ok(r) if r.status().is_success() => info!(user = %id, "forgot: reset link sent"),
            Ok(r) => warn!(user = %id, status = %r.status(), "forgot: Keycloak could not send the reset email"),
            Err(e) => warn!(user = %id, error = %e, "forgot: Keycloak unreachable"),
        }
        answer()
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = (config, idp);
        unavailable("Password reset by email isn't available in this edition. Ask your administrator.")
    }
}

fn unavailable(msg: &str) -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": msg }))).into_response()
}
