//! Sign-in per organisation (.ai/A3T_ORG_SSO_SPEC.md).
//!
//! Every org brings its own identity provider. Keycloak realm `a3t` brokers it
//! (an *identity provider* of the realm, alias = the org's `sso.idp_alias`), so
//! FlowR still trusts one issuer. This module runs the browser side:
//!
//! - `GET /auth/sso/start?org=<handle>|email=<address>[&return=<origin>]` finds
//!   the org, and sends the browser to the realm's authorize endpoint with PKCE
//!   and `kc_idp_hint`, so Keycloak goes straight to the org's provider.
//! - `GET /auth/sso/callback?code&state` exchanges the code (back channel, so
//!   the token response is trusted as sent), then:
//!   * the org is the one whose `idp_alias` is the provider the user actually
//!     signed in through (`identity_provider`) — nothing the enterprise's token
//!     says can name another org (R2);
//!   * the role there comes from their groups (`a3t_groups`) through the org's
//!     policy file `access/org.yaml` (GOV-5; no file, or switched off: no
//!     membership) (R3);
//!   * the membership row made at sign-in (`added_by = "sso"`) is rewritten every
//!     time; rows an admin made (invites) are never touched (R4, R5);
//!   * the A3T token carries that org, and the browser returns to the console
//!     with it in the URL fragment (never sent to a server).
//!
//! Editors (the VS Code extension) sign in the same way, as a native app (RFC
//! 8252): `start` with `redirect_uri` (the editor's own URI, allow-listed) and
//! the editor's `code_challenge` (S256); no org is needed (the realm's sign-in
//! page offers every provider). The callback then sends the browser to the
//! editor with a short-lived signed *handoff code*, not tokens; the editor
//! trades it at `POST /auth/sso/token` with its verifier, which opens the
//! session. A code caught on the way is useless without the verifier.

use std::sync::Arc;

use axum::{
    extract::{Extension, Query},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::auth::{create_refresh_token, create_token, AuthConfig};

const STATE_AUD: &str = "a3t-sso-state";
const STATE_TTL_SECS: usize = 600;
const HANDOFF_AUD: &str = "a3t-sso-handoff";
const HANDOFF_TTL_SECS: usize = 120;
/// URI schemes of the editors the A3T extension runs in.
const EDITOR_SCHEMES: &[&str] = &["vscode", "vscode-insiders", "vscodium", "cursor", "windsurf"];
const EDITOR_EXTENSION: &str = "a3t-ai.a3t";
const SSO_ADDED_BY: &str = "sso";

/// An org as sign-in needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct OrgSso {
    pub id: String,
    pub handle: String,
    pub idp_alias: String,
    pub domains: Vec<String>,
}

/// Orgs with sign-in configured, from `orgs` rows (`settings.sso`).
pub fn orgs_with_sso(rows: &[Value]) -> Vec<OrgSso> {
    let s = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let mut out: Vec<OrgSso> = Vec::new();
    for r in rows {
        let settings: Value = serde_json::from_str(&s(r, "settings")).unwrap_or(Value::Null);
        let Some(sso) = settings.get("sso").filter(|v| v.is_object()) else { continue };
        let alias = s(sso, "idp_alias");
        if alias.is_empty() || out.iter().any(|o| o.id == s(r, "id")) {
            continue;
        }
        out.push(OrgSso {
            id: s(r, "id"),
            handle: s(r, "slug"),
            idp_alias: alias,
            domains: sso.get("domains").and_then(|d| d.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(|d| d.to_ascii_lowercase())).collect())
                .unwrap_or_default(),
        });
    }
    out
}

/// The org a sign-in starts for: by handle, or by the email's domain.
pub fn find_org<'a>(orgs: &'a [OrgSso], org: Option<&str>, email: Option<&str>) -> Option<&'a OrgSso> {
    if let Some(h) = org.map(str::trim).filter(|h| !h.is_empty()) {
        return orgs.iter().find(|o| o.handle == h);
    }
    let domain = email?.rsplit_once('@')?.1.trim().to_ascii_lowercase();
    orgs.iter().find(|o| o.domains.iter().any(|d| *d == domain))
}

/// The org's sign-in mapping, `access/org.yaml` (GOV-5), when it is enforced.
async fn org_access(config: &AuthConfig, org: &OrgSso) -> Option<flowr_policy::policy_file::OrgAccessFile> {
    let q = flowr_ce::handlers::call_lancedb::sql_quote;
    let filter = format!("org = {} AND (path = 'access/org.yaml' OR path = 'access/org.yml')", q(&org.handle));
    let row = platform_rows(config, flowr_ce::org_policy::FILES, Some(&filter)).await.ok()?.into_iter().next()?;
    let path = row.get("path").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let off = platform_rows(config, flowr_ce::org_policy::OFF, Some(&format!("org = {} AND path = {}", q(&org.handle), q(&path)))).await
        .map(|r| !r.is_empty()).unwrap_or(false);
    if off {
        return None;
    }
    match flowr_policy::policy_file::OrgAccessFile::parse(row.get("content").and_then(|v| v.as_str()).unwrap_or_default()) {
        Ok(f) => Some(f),
        Err(e) => {
            warn!(org = %org.handle, error = %e, "sso: access/org.yaml doesn't parse; no org role is given");
            None
        }
    }
}

/// The role a user's groups give them in `org`: from its `access/org.yaml`
/// (the highest mapped role, else its `default`); `None` = no membership.
pub fn role_for(file: Option<&flowr_policy::policy_file::OrgAccessFile>, groups: &[String]) -> Option<String> {
    file?.role_for(groups)
}

/// A claim that may be a string or a list of strings.
fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        _ => vec![],
    }
}

#[derive(Serialize, Deserialize)]
struct SsoState {
    aud: String,
    exp: usize,
    /// PKCE verifier (the state is signed, and only ever travels through the browser
    /// back to us, so it is safe to carry it here instead of in server memory —
    /// every FlowR replica can finish a sign-in another one started).
    pkce: String,
    /// The provider sign-in was started for.
    idp: String,
    /// Where the console is (validated origin).
    ret: String,
    /// An editor's sign-in: where to hand the code back (validated) and the
    /// editor's PKCE challenge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_redirect: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_challenge: Option<String>,
}

/// What an editor's handoff code says: who signed in, as the callback decided,
/// bound to the editor's challenge.
#[derive(Serialize, Deserialize)]
struct Handoff {
    aud: String,
    exp: usize,
    cc: String,
    sub: String,
    org: String,
    role: String,
    email: String,
    name: String,
}

#[derive(Deserialize)]
pub struct StartQuery {
    pub org: Option<String>,
    pub email: Option<String>,
    #[serde(rename = "return")]
    pub ret: Option<String>,
    /// An editor's sign-in: its URI to come back to, and its PKCE challenge.
    pub redirect_uri: Option<String>,
    pub code_challenge: Option<String>,
}

#[derive(Deserialize)]
pub struct TokenBody {
    pub code: String,
    pub code_verifier: String,
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

fn error(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

fn redirect(to: &str) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, to.to_string()), (header::CACHE_CONTROL, "no-store".to_string())]).into_response()
}

/// Origins the browser may be sent back to: this server's own URL, and those in
/// `FLOWR_SSO_RETURN_ORIGINS` (comma-separated). Anything else is the default.
pub(crate) fn return_origin(config: &AuthConfig, asked: Option<&str>) -> String {
    let own = config.resource_uri.trim_end_matches('/').to_string();
    let allowed: Vec<String> = std::env::var("FLOWR_SSO_RETURN_ORIGINS").unwrap_or_default()
        .split(',').map(|s| s.trim().trim_end_matches('/').to_string()).filter(|s| !s.is_empty())
        .chain(std::iter::once(own.clone())).collect();
    asked.map(|a| a.trim().trim_end_matches('/').to_string())
        .filter(|a| allowed.iter().any(|x| x == a))
        .unwrap_or(own)
}

/// An editor URI the sign-in may hand a code to: `vscode://a3t-ai.a3t/auth`
/// (and the same in VS Code's forks), or `https://` under an allowed return
/// origin (a browser-hosted VS Code, `FLOWR_SSO_RETURN_ORIGINS`). Anything else
/// would let a link send someone's sign-in to a stranger.
pub(crate) fn editor_redirect_ok(config: &AuthConfig, uri: &str) -> bool {
    editor_redirect_with(uri, |origin| return_origin(config, Some(origin)) == origin)
}

fn editor_redirect_with(uri: &str, origin_allowed: impl Fn(&str) -> bool) -> bool {
    let Ok(u) = reqwest::Url::parse(uri) else { return false };
    if EDITOR_SCHEMES.contains(&u.scheme()) {
        return u.host_str() == Some(EDITOR_EXTENSION) && u.path() == "/auth";
    }
    if u.scheme() != "https" {
        return false;
    }
    origin_allowed(&u.origin().ascii_serialization())
}

/// S256 of a PKCE verifier, as sent in `code_challenge`.
fn s256(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn with_query(uri: &str, k: &str, v: &str) -> String {
    format!("{uri}{}{k}={}", if uri.contains('?') { '&' } else { '?' }, urlenc(v))
}

fn callback_uri(config: &AuthConfig) -> String {
    format!("{}/auth/sso/callback", config.resource_uri.trim_end_matches('/'))
}

fn realm_endpoint(config: &AuthConfig, path: &str) -> Option<String> {
    let iss = config.oidc_issuer.trim_end_matches('/');
    (!iss.is_empty()).then(|| format!("{iss}/protocol/openid-connect/{path}"))
}

async fn platform_rows(config: &AuthConfig, table: &str, filter: Option<&str>) -> Result<Vec<Value>, String> {
    let db = config.platform.get().await;
    db.query(table, filter, 10_000).await.map(|b| flowr_ce::handlers::call_lancedb::batches_to_json(&b))
}

/// `GET /auth/sso/start`
pub async fn start_handler(Extension(config): Extension<Arc<AuthConfig>>, Query(q): Query<StartQuery>) -> Response {
    let Some(authorize) = realm_endpoint(&config, "auth") else {
        return error(StatusCode::NOT_FOUND, "Single sign-on is not configured.");
    };
    let editor = match (q.redirect_uri.as_deref(), q.code_challenge.as_deref()) {
        (None, None) => None,
        (Some(r), Some(c)) if editor_redirect_ok(&config, r) && c.len() == 43 && c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') => Some((r.to_string(), c.to_string())),
        (Some(r), _) if !editor_redirect_ok(&config, r) => return error(StatusCode::BAD_REQUEST, "This editor cannot sign in here (redirect_uri not allowed)."),
        _ => return error(StatusCode::BAD_REQUEST, "An editor's sign-in needs redirect_uri and an S256 code_challenge."),
    };
    // An editor without an org or email: the realm's own sign-in page, which
    // offers every provider.
    let org = if editor.is_some() && q.org.is_none() && q.email.is_none() {
        None
    } else {
        let orgs = match platform_rows(&config, "orgs", None).await {
            Ok(rows) => orgs_with_sso(&rows),
            Err(e) => { warn!(error = %e, "sso: orgs unreadable"); return error(StatusCode::SERVICE_UNAVAILABLE, "Organisations are unavailable; try again."); }
        };
        match find_org(&orgs, q.org.as_deref(), q.email.as_deref()) {
            Some(o) => Some(o.clone()),
            None => return error(StatusCode::NOT_FOUND, "No organisation signs in that way. Check the organisation name or your work email."),
        }
    };
    let pkce = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(pkce.as_bytes()));
    let state = SsoState {
        aud: STATE_AUD.into(),
        exp: chrono::Utc::now().timestamp() as usize + STATE_TTL_SECS,
        pkce,
        idp: org.as_ref().map(|o| o.idp_alias.clone()).unwrap_or_default(),
        ret: return_origin(&config, q.ret.as_deref()),
        client_redirect: editor.as_ref().map(|e| e.0.clone()),
        client_challenge: editor.as_ref().map(|e| e.1.clone()),
    };
    let state = match encode(&Header::default(), &state, &EncodingKey::from_secret(config.jwt_secret.as_bytes())) {
        Ok(s) => s,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("state: {e}")),
    };
    let mut url = reqwest::Url::parse(&authorize).expect("issuer is a URL");
    url.query_pairs_mut()
        .append_pair("client_id", &config.oidc_client_id)
        .append_pair("response_type", "code")
        .append_pair("scope", "openid email profile")
        .append_pair("redirect_uri", &callback_uri(&config))
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    if let Some(org) = &org {
        url.query_pairs_mut().append_pair("kc_idp_hint", &org.idp_alias);
    }
    if let Some(e) = q.email.as_deref().filter(|e| e.contains('@')) {
        url.query_pairs_mut().append_pair("login_hint", e);
    }
    info!(org = %org.as_ref().map(|o| o.handle.as_str()).unwrap_or("-"), editor = editor.is_some(), "sso: sign-in started");
    redirect(url.as_str())
}

/// `GET /auth/sso/callback`
pub async fn callback_handler(Extension(config): Extension<Arc<AuthConfig>>, Query(q): Query<CallbackQuery>) -> Response {
    let mut v = Validation::default();
    v.set_audience(&[STATE_AUD]);
    let state = match q.state.as_deref().map(|s| decode::<SsoState>(s, &DecodingKey::from_secret(config.jwt_secret.as_bytes()), &v)) {
        Some(Ok(s)) => s.claims,
        _ => return error(StatusCode::BAD_REQUEST, "This sign-in link is invalid or has expired. Start again."),
    };
    let back = |msg: &str| match &state.client_redirect {
        Some(r) => redirect(&with_query(r, "error", msg)),
        None => redirect(&format!("{}/#sso_error={}", state.ret, urlenc(msg))),
    };
    if let Some(e) = q.error.as_deref() {
        return back(q.error_description.as_deref().unwrap_or(e));
    }
    let Some(code) = q.code.as_deref() else { return back("The sign-in returned no code.") };
    let Some(token_url) = realm_endpoint(&config, "token") else { return back("Single sign-on is not configured.") };

    // Back channel: the token response comes straight from the realm over TLS,
    // so its claims are taken as sent (OIDC Core §3.1.3.7).
    let resp = reqwest::Client::new().post(&token_url).form(&[
        ("grant_type", "authorization_code"),
        ("client_id", config.oidc_client_id.as_str()),
        ("code", code),
        ("redirect_uri", &callback_uri(&config)),
        ("code_verifier", state.pkce.as_str()),
    ]).send().await;
    let body: Value = match resp {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or(Value::Null),
        Ok(r) => { warn!(status = %r.status(), "sso: code exchange refused"); return back("The sign-in could not be completed. Start again."); }
        Err(e) => { warn!(error = %e, "sso: token endpoint unreachable"); return back("The sign-in service is unreachable. Try again."); }
    };
    let Some(claims) = body.get("access_token").and_then(|t| t.as_str()).and_then(jwt_payload) else {
        return back("The sign-in returned no token.");
    };

    let s = |k: &str| claims.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let sub = s("sub");
    if sub.is_empty() {
        return back("The sign-in returned no user.");
    }
    let email = s("email");
    let name = Some(s("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| s("preferred_username"));
    let realm_roles = strings(claims.get("realm_access").and_then(|r| r.get("roles")));
    let via = s("identity_provider");
    let groups = strings(claims.get("a3t_groups"));

    let orgs = platform_rows(&config, "orgs", None).await.map(|r| orgs_with_sso(&r)).unwrap_or_default();
    // R2: the org is the one whose provider the user signed in through.
    let home = orgs.iter().find(|o| !via.is_empty() && o.idp_alias == via);
    if !state.idp.is_empty() && via != state.idp {
        warn!(started = %state.idp, via = %via, user = %sub, "sso: signed in through a different provider than the one asked for");
    }
    let mut org_id = "org-default".to_string();
    if let Some(org) = home {
        org_id = org.id.clone();
        let file = org_access(&config, org).await;
        let role = role_for(file.as_ref(), &groups);
        if let Err(e) = sync_membership(&config, org, &sub, &email, &name, role.as_deref()).await {
            warn!(error = %e, org = %org.handle, user = %sub, "sso: membership not updated");
            return back("Your organisation membership could not be updated. Try again.");
        }
        info!(org = %org.handle, user = %sub, email = %email, role = ?role, groups = ?groups, "sso: signed in");
    } else if !via.is_empty() {
        warn!(via = %via, user = %sub, "sso: provider belongs to no organisation");
    }

    let role = crate::auth_routes::pick_role(&realm_roles);
    // An editor: hand it a code for its verifier; the session opens when it trades it.
    if let (Some(to), Some(cc)) = (&state.client_redirect, &state.client_challenge) {
        let h = Handoff {
            aud: HANDOFF_AUD.into(),
            exp: chrono::Utc::now().timestamp() as usize + HANDOFF_TTL_SECS,
            cc: cc.clone(), sub, org: org_id, role, email, name,
        };
        return match encode(&Header::default(), &h, &EncodingKey::from_secret(config.jwt_secret.as_bytes())) {
            Ok(code) => redirect(&with_query(to, "code", &code)),
            Err(e) => back(&format!("handoff: {e}")),
        };
    }
    let (sid, jti, ends) = match crate::sessions::open(&config, &sub, "signin", config.refresh_expiry_secs).await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, user = %sub, "sso: session not recorded");
            return back("Sign-in could not be completed. Try again.");
        }
    };
    let access = match create_token(&config, &sid, &sub, &org_id, &role, &email, &name) {
        Ok(t) => t,
        Err(e) => return back(&e),
    };
    let refresh = match create_refresh_token(&config, &sub, &sid, &jti, ends) {
        Ok(t) => t,
        Err(e) => return back(&e),
    };
    redirect(&format!("{}/#sso={}&sso_refresh={}&expires_in={}", state.ret, access, refresh, config.token_expiry_secs))
}

/// `POST /auth/sso/token` `{ code, code_verifier }`: an editor trades its
/// handoff code for a session (the same tokens as `/auth/login`).
pub async fn token_handler(Extension(config): Extension<Arc<AuthConfig>>, Json(b): Json<TokenBody>) -> Response {
    let mut v = Validation::default();
    v.set_audience(&[HANDOFF_AUD]);
    let Ok(h) = decode::<Handoff>(&b.code, &DecodingKey::from_secret(config.jwt_secret.as_bytes()), &v).map(|d| d.claims) else {
        return error(StatusCode::BAD_REQUEST, "This sign-in code is invalid or has expired. Sign in again.");
    };
    if s256(&b.code_verifier) != h.cc {
        warn!(user = %h.sub, "sso: handoff code traded with the wrong verifier");
        return error(StatusCode::BAD_REQUEST, "This sign-in code was not issued to this editor.");
    }
    info!(user = %h.sub, email = %h.email, "sso: editor signed in");
    crate::auth_routes::issue_session(&config, (h.sub, h.org, h.role, h.email, h.name)).await
}

/// Rewrite the membership made at sign-in; an invite (any row not made by
/// sign-in) is left exactly as its admin set it.
async fn sync_membership(config: &AuthConfig, org: &OrgSso, user: &str, email: &str, name: &str, role: Option<&str>) -> Result<(), String> {
    let id = format!("{}:{}", org.id, user);
    let esc = |x: &str| x.replace('"', "").replace('\\', "");
    let filter = format!("id = \"{}\"", esc(&id));
    let existing = platform_rows(config, "org_members", Some(&filter)).await?;
    if existing.iter().any(|r| r.get("added_by").and_then(|a| a.as_str()) != Some(SSO_ADDED_BY)) {
        return Ok(());
    }
    let same = existing.len() == 1 && role.is_some_and(|r| existing[0].get("role").and_then(|x| x.as_str()) == Some(r));
    if same {
        return Ok(());
    }
    let db = config.platform.get().await;
    if !existing.is_empty() {
        db.delete("org_members", &filter).await?;
    }
    if let Some(role) = role {
        db.insert("org_members", &[json!({
            "id": id, "org_id": org.id, "user_id": user, "email": email, "name": name,
            "role": role, "added_by": SSO_ADDED_BY, "created_at": chrono::Utc::now().to_rfc3339(),
        })]).await?;
    }
    // This pod decides with the new membership at once.
    flowr_ce::access::invalidate();
    Ok(())
}

fn jwt_payload(token: &str) -> Option<Value> {
    let b = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(token.split('.').nth(1)?).ok()?;
    serde_json::from_slice(&b).ok()
}

fn urlenc(s: &str) -> String {
    reqwest::Url::parse_with_params("x:", &[("v", s)]).ok()
        .and_then(|u| u.query().map(|q| q.trim_start_matches("v=").to_string()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Value> {
        vec![
            json!({"id": "org-acme", "slug": "acme", "settings": json!({"sso": {"idp_alias": "acme", "domains": ["acme.test"], "role_map": {"a3t-admins": "admin", "a3t-sec": "securityadmin"}, "default_role": "member"}}).to_string()}),
            json!({"id": "org-beta", "slug": "beta", "settings": json!({"runtime": "shared"}).to_string()}),
            json!({"id": "org-gamma", "slug": "gamma", "settings": json!({"sso": {"idp_alias": "gamma", "role_map": {}, "default_role": ""}}).to_string()}),
        ]
    }

    #[test]
    fn editors_get_codes_only_at_their_own_uri() {
        let allowed = |o: &str| o == "https://code.example.test";
        for ok in ["vscode://a3t-ai.a3t/auth", "cursor://a3t-ai.a3t/auth", "vscode-insiders://a3t-ai.a3t/auth?windowId=3",
                   "https://code.example.test/callback?vscode-scheme=vscode&vscode-authority=a3t-ai.a3t"] {
            assert!(editor_redirect_with(ok, allowed), "{ok}");
        }
        for bad in ["vscode://evil.ext/auth", "vscode://a3t-ai.a3t/other", "https://evil.test/a3t-ai.a3t/auth",
                    "http://code.example.test/x", "javascript:alert(1)", "not a uri"] {
            assert!(!editor_redirect_with(bad, allowed), "{bad}");
        }
    }

    #[test]
    fn the_verifier_must_match_the_challenge() {
        // Base64url (no padding) of SHA-256, as computed by Python's hashlib.
        assert_eq!(s256("a3t-editor-verifier-0123456789-abcdefghijklmnopqrs"), "ddJxNpng3Wg9_hzPtkNDUNrYyl1nMaf8iZ_GjpTcoAU");
        assert_eq!(with_query("vscode://a3t-ai.a3t/auth", "code", "a b"), "vscode://a3t-ai.a3t/auth?code=a+b");
        assert_eq!(with_query("https://x.test/cb?s=1", "error", "no"), "https://x.test/cb?s=1&error=no");
    }

    #[test]
    fn only_orgs_with_a_provider_sign_in() {
        let o = orgs_with_sso(&rows());
        assert_eq!(o.iter().map(|x| x.handle.as_str()).collect::<Vec<_>>(), ["acme", "gamma"]);
        assert_eq!(find_org(&o, Some("acme"), None).unwrap().id, "org-acme");
        assert_eq!(find_org(&o, None, Some("Jane@ACME.test")).unwrap().id, "org-acme");
        assert!(find_org(&o, Some("beta"), None).is_none());
        assert!(find_org(&o, None, Some("x@other.test")).is_none());
    }

    #[test]
    fn groups_map_through_the_org_access_file() {
        let f = flowr_policy::policy_file::OrgAccessFile::parse("kind: a3t/org-access\ngroups: { a3t-admins: admin, a3t-sec: securityadmin }\ndefault: member\n").unwrap();
        assert_eq!(role_for(Some(&f), &["a3t-admins".into(), "staff".into()]).as_deref(), Some("admin"));
        assert_eq!(role_for(Some(&f), &["/a3t-sec".into()]).as_deref(), Some("securityadmin"));
        assert_eq!(role_for(Some(&f), &["staff".into()]).as_deref(), Some("member"));
        assert_eq!(role_for(None, &["a3t-admins".into()]), None, "no file: no membership");
    }
}
