//! Auth HTTP routes — login, signup, refresh, me
//!
//! These endpoints are PUBLIC (no JWT required).
//! They authenticate against the A3T users table in LanceDB.

use axum::{
    extract::Extension,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

use crate::auth::{
    authenticate_user, create_refresh_token, create_token, extract_bearer_token, hash_password,
    validate_token, AuthConfig,
};

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct SignupRequest {
    pub name: String,
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct AuthResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub user: UserInfo,
}

#[derive(Serialize)]
pub struct UserInfo {
    pub id: String,
    pub org_id: String,
    pub email: String,
    pub name: String,
    pub role: String,
}

/// POST /auth/login
pub async fn login_handler(
    Extension(config): Extension<Arc<AuthConfig>>,
    Json(body): Json<LoginRequest>,
) -> impl IntoResponse {
    if body.email.is_empty() || body.password.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Email and password required" })),
        ).into_response();
    }

    // Resolve the identity: try the local A3T user store first (superadmin, local
    // signups); on failure fall back to Keycloak (EE) so SSO-provisioned users can
    // sign in through the same email/password form. `(user_id, org, role, email, name)`.
    let resolved = match authenticate_user(config.platform.get().await.as_ref(), &body.email, &body.password).await {
        Ok(t) => Some(t),
        Err(local_err) => match keycloak_login(&config, &body.email, &body.password).await {
            KcLogin::Ok(t) => Some(t),
            // The password was right; the account still has a temporary password.
            // The console asks for a new one and posts it to /auth/first-password.
            KcLogin::SetupRequired => {
                info!(email = %body.email, "Login: a new password is needed first");
                return (StatusCode::FORBIDDEN, Json(json!({
                    "error": "Choose a new password to finish setting up your account.",
                    "code": "password_change_required",
                }))).into_response();
            }
            KcLogin::Failed => {
                warn!(email = %body.email, error = %local_err, "Login failed (local + oidc)");
                None
            }
        },
    };

    let Some(identity) = resolved else {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "Invalid credentials" }))).into_response();
    };
    issue_session(&config, identity)
}

/// The tokens and user for a signed-in identity `(user_id, org, role, email, name)`.
pub(crate) fn issue_session(config: &AuthConfig, (user_id, org_id, role, email, name): (String, String, String, String, String)) -> axum::response::Response {
    let access_token = match create_token(config, &user_id, &org_id, &role, &email, &name) {
        Ok(t) => t,
        Err(e) => {
            warn!(error = %e, "Failed to create token");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response();
        }
    };
    let refresh_token = match create_refresh_token(config, &user_id) {
        Ok(t) => t,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
    };

    info!(user = %user_id, email = %email, role = %role, "Login successful");
    Json(AuthResponse {
        access_token,
        refresh_token,
        token_type: "Bearer".to_string(),
        expires_in: config.token_expiry_secs,
        user: UserInfo { id: user_id, org_id, email, name, role },
    }).into_response()
}

/// Split an OIDC issuer (`http://kc:8080/realms/a3t`) into (base_url, realm).
fn split_issuer(issuer: &str) -> Option<(String, String)> {
    let s = issuer.trim_end_matches('/');
    let idx = s.find("/realms/")?;
    let base = s[..idx].to_string();
    let realm = s[idx + "/realms/".len()..].to_string();
    if base.is_empty() || realm.is_empty() { None } else { Some((base, realm)) }
}

/// Map a user's Keycloak realm roles to a single A3T role for menu/RBAC gating.
pub(crate) fn pick_role(roles: &[String]) -> String {
    let has = |r: &str| roles.iter().any(|x| x.eq_ignore_ascii_case(r));
    if has("superadmin") { "superadmin" }
    else if has("securityadmin") { "securityadmin" }
    else if has("admin") { "admin" }
    else if has("operator") || has("developer") { "operator" }
    else { "viewer" }.to_string()
}

/// What a Keycloak password sign-in came to.
pub(crate) enum KcLogin {
    /// Signed in: `(user_id, org, role, email, name)`.
    Ok((String, String, String, String, String)),
    /// The password was right, but the account must set a new one first.
    SetupRequired,
    /// Wrong password, unknown user, or no Keycloak here.
    Failed,
}

/// EE login fallback: authenticate against Keycloak (password grant) and return an
/// A3T identity tuple.
pub(crate) async fn keycloak_login(
    config: &AuthConfig,
    email: &str,
    password: &str,
) -> KcLogin {
    #[cfg(feature = "ee")]
    {
        if config.oidc_issuer.is_empty() { return KcLogin::Failed; }
        let Some((base, realm)) = split_issuer(&config.oidc_issuer) else { return KcLogin::Failed };
        let client = reqwest::Client::new();
        match aesop_tower::oidc_admin::password_login(&client, &base, &realm, &config.oidc_client_id, email, password).await {
            Ok(id) => {
                let role = pick_role(&id.roles);
                let user_id = if id.sub.is_empty() { format!("kc-{email}") } else { id.sub.clone() };
                let out_email = id.email.clone().unwrap_or_else(|| email.to_string());
                let name = id.name.clone().unwrap_or_else(|| id.username.clone());
                info!(user = %user_id, email = %out_email, role = %role, "Login successful (oidc)");
                KcLogin::Ok((user_id, "org-default".to_string(), role, out_email, name))
            }
            Err(aesop_tower::oidc_admin::IdpAdminError::SetupRequired) => KcLogin::SetupRequired,
            Err(e) => { warn!(email = %email, error = %e, "OIDC login failed"); KcLogin::Failed }
        }
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = (config, email, password);
        KcLogin::Failed
    }
}

/// POST /auth/signup
pub async fn signup_handler(
    Extension(config): Extension<Arc<AuthConfig>>,
    Json(body): Json<SignupRequest>,
) -> impl IntoResponse {
    // Where access is enforced, accounts come from approved early access or an
    // administrator (Security › Identity), never from an open form. An install
    // that wants open sign-up says so: FLOWR_AUTH_SIGNUP=open.
    let open = std::env::var("FLOWR_AUTH_SIGNUP").map(|v| v.eq_ignore_ascii_case("open")).unwrap_or(false);
    if !open && flowr_policy::namespace::namespace_guard().enforcing() {
        return (StatusCode::FORBIDDEN, Json(json!({
            "error": "Sign-up is closed. Request early access, or ask your administrator for an account.",
            "code": "signup_closed",
        }))).into_response();
    }
    if body.name.is_empty() || body.email.is_empty() || body.password.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "All fields required" }))).into_response();
    }
    if body.password.len() < 8 {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Password must be at least 8 characters" }))).into_response();
    }
    if !body.email.contains('@') {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid email" }))).into_response();
    }

    // Check if email already exists (platform database: embedded LanceDB or ADB)
    let filter = format!("email = '{}'", body.email.replace('\'', "''"));
    let platform = config.platform.get().await;
    let existing: Vec<arrow_array::RecordBatch> = match platform.query("users", Some(&filter), 1).await {
        Ok(b) => b,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Query error: {}", e) }))).into_response();
        }
    };

    if !existing.is_empty() && existing[0].num_rows() > 0 {
        return (StatusCode::CONFLICT, Json(json!({ "error": "Email already registered" }))).into_response();
    }

    // Create user
    let user_id = format!("usr-{}", uuid_short());
    let org_id = "org-default".to_string(); // Default org for now
    let role = "viewer".to_string(); // New signups get viewer role
    let now = chrono::Utc::now().to_rfc3339();
    let password_hash = hash_password(&body.password);

    let record = json!({
        "id": user_id, "org_id": org_id, "email": body.email, "name": body.name,
        "password_hash": password_hash, "role": role, "avatar": "", "status": "active",
        "last_login_at": "", "created_at": now, "updated_at": now,
    });
    if let Err(e) = platform.insert("users", &[record]).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Insert error: {}", e) }))).into_response();
    }

    // Generate tokens
    let access_token = match create_token(&config, &user_id, &org_id, &role, &body.email, &body.name) {
        Ok(t) => t,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response();
        }
    };
    let refresh_token = match create_refresh_token(&config, &user_id) {
        Ok(t) => t,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response();
        }
    };

    info!(user = %user_id, email = %body.email, "Signup successful");

    (StatusCode::CREATED, Json(AuthResponse {
        access_token,
        refresh_token,
        token_type: "Bearer".to_string(),
        expires_in: config.token_expiry_secs,
        user: UserInfo { id: user_id, org_id, email: body.email, name: body.name, role },
    })).into_response()
}

/// GET /auth/me — returns current user from JWT claims
/// Return the caller's identity from their bearer token.
///
/// This route is merged outside the authenticated router, so it never passes
/// through `jwt_auth_middleware` and cannot rely on `Claims` being present in
/// request extensions — depending on that extractor made every call fail with
/// 500 instead of 401. The token is therefore validated here directly.
pub async fn me_handler(
    Extension(config): Extension<Arc<AuthConfig>>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let token = match extract_bearer_token(&headers) {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Missing or invalid Authorization header. Use: Bearer <token>" })),
            )
                .into_response();
        }
    };

    match validate_token(&token, &config.jwt_secret, &config.resource_uri) {
        Ok(claims) => Json(UserInfo {
            id: claims.sub,
            org_id: claims.org,
            email: claims.email,
            name: claims.name,
            role: claims.role,
        })
        .into_response(),
        Err(e) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": format!("Invalid token: {}", e) })),
        )
            .into_response(),
    }
}

fn uuid_short() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    format!("{:x}", t)
}
