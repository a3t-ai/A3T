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
    /// ID-9: the authenticator code, for accounts whose sign-in needs one
    /// (SuperAdmin and SecurityAdmin).
    #[serde(default)]
    pub otp: Option<String>,
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
    Extension(idp): Extension<crate::IdpAdminState>,
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
        // ID-9: SuperAdmin and SecurityAdmin sign in through the realm, which asks
        // for their second factor; a local password alone never makes one.
        Ok(t) if mfa_role(&t.2) && !config.oidc_issuer.is_empty() => {
            warn!(email = %body.email, role = %t.2, "Login: a local password for a role that needs a second factor was refused");
            return (StatusCode::FORBIDDEN, Json(json!({
                "error": "This account signs in through A3T's sign-in, with its authenticator code.",
                "code": "mfa_required",
            }))).into_response();
        }
        Ok(t) => Some((t, crate::auth::SignIn::now())),
        Err(local_err) => match keycloak_login(&config, &body.email, &body.password, body.otp.as_deref()).await {
            KcLogin::Ok(t, mut si) => {
                // A code proves a second factor only for an account that has
                // an authenticator: the realm ignores a code from anyone else.
                si.mfa = si.mfa || (body.otp.as_deref().is_some_and(|o| !o.trim().is_empty()) && has_otp(&idp, &t.0).await);
                Some((t, si))
            }
            // The password was right; the account still has a temporary password.
            // The console asks for a new one and posts it to /auth/first-password.
            KcLogin::SetupRequired => {
                // An authenticator to set up is a pending action too: the realm's
                // own page says which one.
                if password_step(&config, &body.email, &body.password).await == Some(PasswordStep::SetUpAuthenticator) {
                    return otp_setup_required(&body.email);
                }
                info!(email = %body.email, "Login: a new password is needed first");
                return (StatusCode::FORBIDDEN, Json(json!({
                    "error": "Choose a new password to finish setting up your account.",
                    "code": "password_change_required",
                }))).into_response();
            }
            KcLogin::Failed => {
                // A password grant answers a wrong password and a missing or wrong
                // code alike. The realm's sign-in page checks the password first:
                // only once it was right does the console ask for the code, as a
                // step of its own.
                let with_code = body.otp.as_deref().is_some_and(|o| !o.trim().is_empty());
                match password_step(&config, &body.email, &body.password).await {
                    Some(PasswordStep::CodeNext) if with_code => {
                        info!(email = %body.email, "Login: the authenticator code was wrong");
                        return (StatusCode::UNAUTHORIZED, Json(json!({
                            "error": "That code didn't work. Enter the code your app shows now; it changes every 30 seconds.",
                            "code": "otp_invalid",
                        }))).into_response();
                    }
                    Some(PasswordStep::CodeNext) => {
                        info!(email = %body.email, "Login: password right, authenticator code next");
                        return (StatusCode::UNAUTHORIZED, Json(json!({
                            "error": "Enter the 6-digit code from your authenticator app.",
                            "code": "otp_required",
                        }))).into_response();
                    }
                    Some(PasswordStep::SetUpAuthenticator) => return otp_setup_required(&body.email),
                    Some(PasswordStep::NewPassword) => {
                        return (StatusCode::FORBIDDEN, Json(json!({
                            "error": "Choose a new password to finish setting up your account.",
                            "code": "password_change_required",
                        }))).into_response();
                    }
                    _ => {}
                }
                warn!(email = %body.email, error = %local_err, "Login failed (local + oidc)");
                None
            }
        },
    };

    let Some((identity, si)) = resolved else {
        return (StatusCode::UNAUTHORIZED, Json(json!({
            "error": "Email or password is wrong.",
            "code": "invalid_credentials",
        }))).into_response();
    };
    issue_session(&config, identity, si).await
}

/// Whether Keycloak user `sub` has an authenticator set up (false when it
/// can't be read: no second factor is claimed on a guess).
async fn has_otp(idp: &crate::IdpAdminState, sub: &str) -> bool {
    #[cfg(feature = "ee")]
    {
        use a3t_security::oidc_admin as kc;
        let Some(cfg) = kc::IdpAdminConfig::from_issuer(&idp.issuer, idp.admin_user.clone(), idp.admin_password.clone()) else { return false };
        let Ok(token) = kc::admin_token(&idp.client, &cfg).await else { return false };
        match kc::has_otp(&idp.client, &cfg, &token, sub).await {
            Ok(b) => b,
            Err(e) => {
                warn!(error = %e, user = %sub, "login: authenticator not readable; no second factor recorded");
                false
            }
        }
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = (idp, sub);
        false
    }
}

/// Sign in a verified identity `(user_id, org, role, email, name)`: a new
/// session (sessions.rs) and its tokens.
pub(crate) async fn issue_session(config: &AuthConfig, identity: (String, String, String, String, String), si: crate::auth::SignIn) -> axum::response::Response {
    let (sid, jti, ends) = match crate::sessions::open(config, &identity.0, "signin", config.refresh_expiry_secs).await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "sign-in: session not recorded");
            return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "Try again in a moment." }))).into_response();
        }
    };
    info!(user = %identity.0, email = %identity.3, role = %identity.2, "Login successful");
    session_tokens(config, identity, &sid, &jti, ends, Some(&si))
}

/// The tokens and user for session `sid` with refresh token `jti`.
pub(crate) fn session_tokens(config: &AuthConfig, (user_id, org_id, role, email, name): (String, String, String, String, String), sid: &str, jti: &str, ends: i64, si: Option<&crate::auth::SignIn>) -> axum::response::Response {
    let access_token = match create_token(config, sid, &user_id, &org_id, &role, &email, &name, si) {
        Ok(t) => t,
        Err(e) => {
            warn!(error = %e, "Failed to create token");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response();
        }
    };
    let refresh_token = match create_refresh_token(config, &user_id, sid, jti, ends, si) {
        Ok(t) => t,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
    };
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
    else if has("operator") { "operator" }
    else { "viewer" }.to_string()
}

/// The account must set up an authenticator before it can sign in (ID-9).
fn otp_setup_required(email: &str) -> axum::response::Response {
    info!(email = %email, "Login: password right, an authenticator must be set up first");
    (StatusCode::FORBIDDEN, Json(json!({
        "error": "Set up an authenticator app to finish signing in.",
        "code": "otp_setup_required",
    }))).into_response()
}

#[cfg(feature = "ee")]
use a3t_security::oidc_admin::PasswordStep;
/// What the sign-in realm asks after this password (CE: no realm, `None`).
#[cfg(not(feature = "ee"))]
#[derive(PartialEq, Eq)]
#[allow(dead_code)]
enum PasswordStep { CodeNext, SetUpAuthenticator, NewPassword }

/// Whether the realm's own sign-in page takes this password and what it asks
/// next (`None`: no realm here, or its page couldn't be read).
async fn password_step(config: &AuthConfig, email: &str, password: &str) -> Option<PasswordStep> {
    #[cfg(feature = "ee")]
    {
        let (base, realm) = split_issuer(&config.oidc_issuer)?;
        let step = a3t_security::oidc_admin::password_step(&base, &realm, &config.oidc_client_id, &crate::sso::callback_uri(config), email, password).await;
        if step == PasswordStep::Unknown {
            warn!(email = %email, "Login: the realm's sign-in page could not be read; no code step offered");
            return None;
        }
        Some(step)
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = (config, email, password);
        None
    }
}

/// What a Keycloak password sign-in came to.
pub(crate) enum KcLogin {
    /// Signed in: `(user_id, org, role, email, name)`, and how.
    Ok((String, String, String, String, String), crate::auth::SignIn),
    /// The password was right, but the account must set a new one first.
    SetupRequired,
    /// Wrong password, unknown user, or no Keycloak here.
    Failed,
}

/// EE login fallback: authenticate against Keycloak (password grant) and return an
/// A3T identity tuple.
/// GET /auth/account — the sign-in realm's own account page (set up an
/// authenticator, ID-9; see your sessions), when FlowR signs in through one.
pub async fn account_handler(Extension(config): Extension<Arc<AuthConfig>>) -> axum::response::Response {
    if config.oidc_issuer.is_empty() {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "This FlowR has no sign-in realm." }))).into_response();
    }
    let url = format!("{}/account/#/security/signingin", config.oidc_issuer.trim_end_matches('/'));
    axum::response::Redirect::temporary(&url).into_response()
}

/// Roles whose sign-in needs a second factor (ID-9).
pub(crate) fn mfa_role(role: &str) -> bool {
    matches!(role, "superadmin" | "securityadmin")
}

pub(crate) async fn keycloak_login(
    config: &AuthConfig,
    email: &str,
    password: &str,
    otp: Option<&str>,
) -> KcLogin {
    #[cfg(feature = "ee")]
    {
        if config.oidc_issuer.is_empty() { return KcLogin::Failed; }
        let Some((base, realm)) = split_issuer(&config.oidc_issuer) else { return KcLogin::Failed };
        let client = reqwest::Client::new();
        match a3t_security::oidc_admin::password_login(&client, &base, &realm, &config.oidc_client_id, email, password, otp).await {
            Ok(id) => {
                let role = pick_role(&id.roles);
                let user_id = if id.sub.is_empty() { format!("kc-{email}") } else { id.sub.clone() };
                let out_email = id.email.clone().unwrap_or_else(|| email.to_string());
                let name = id.name.clone().unwrap_or_else(|| id.username.clone());
                info!(user = %user_id, email = %out_email, role = %role, "Login successful (oidc)");
                KcLogin::Ok((user_id, "org-default".to_string(), role, out_email, name), crate::auth::SignIn { mfa: id.second_factor, ..crate::auth::SignIn::now() })
            }
            Err(a3t_security::oidc_admin::IdpAdminError::SetupRequired) => KcLogin::SetupRequired,
            Err(e) => { warn!(email = %email, error = %e, "OIDC login failed"); KcLogin::Failed }
        }
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = (config, email, password, otp);
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

    info!(user = %user_id, email = %body.email, "Signup successful");
    let mut res = issue_session(&config, (user_id, org_id, role, body.email, body.name), crate::auth::SignIn::now()).await;
    if res.status() == StatusCode::OK {
        *res.status_mut() = StatusCode::CREATED;
    }
    res
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

    match validate_token(&token, &config) {
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

#[derive(Deserialize, Default)]
pub struct LogoutBody {
    #[serde(default)]
    pub refresh_token: Option<String>,
}

/// `POST /auth/logout` — end the session named by the bearer token and/or the
/// refresh token in the body, on every pod (ID-6). Its tokens stop working here
/// at once and on other pods within `sessions::RELOAD_SECS`.
pub async fn logout_handler(
    Extension(config): Extension<Arc<AuthConfig>>,
    headers: axum::http::HeaderMap,
    body: Option<Json<LogoutBody>>,
) -> axum::response::Response {
    let mut sids: Vec<String> = Vec::new();
    let bearer = extract_bearer_token(&headers);
    if let Some(sid) = bearer.as_deref().and_then(|t| crate::auth::session_of(t, &config)) {
        sids.push(sid);
    }
    // AUD-3: ending a borrowed session is Act as stopping.
    if let Some(claims) = bearer.as_deref().and_then(|t| crate::auth::validate_token(t, &config).ok()).filter(|c| c.act.is_some()) {
        let p = crate::auth::principal_for(&claims);
        flowr_policy::audit::event(&p, "auth:impersonate_stop", "User", &claims.sub, None, None, None, Some("acting stopped"));
    }
    if let Some(rt) = body.and_then(|b| b.0.refresh_token).and_then(|t| crate::auth::validate_refresh(&t, &config, true).ok()) {
        if !sids.contains(&rt.sid) {
            sids.push(rt.sid);
        }
    }
    for sid in &sids {
        if let Err(e) = crate::sessions::revoke(&config, sid, "signed out").await {
            warn!(error = %e, "sign-out: session not revoked");
            return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "Try again in a moment." }))).into_response();
        }
    }
    Json(json!({ "ok": true, "ended": sids.len() })).into_response()
}
