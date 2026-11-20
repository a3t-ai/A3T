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
//!     `role_map`, else `default_role` (R3);
//!   * the membership row made at sign-in (`added_by = "sso"`) is rewritten every
//!     time; rows an admin made (invites) are never touched (R4, R5);
//!   * the A3T token carries that org, and the browser returns to the console
//!     with it in the URL fragment (never sent to a server).

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
const SSO_ADDED_BY: &str = "sso";

/// An org as sign-in needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct OrgSso {
    pub id: String,
    pub handle: String,
    pub idp_alias: String,
    pub domains: Vec<String>,
    pub role_map: Vec<(String, String)>,
    pub default_role: String,
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
            role_map: sso.get("role_map").and_then(|m| m.as_object())
                .map(|m| m.iter().filter_map(|(g, r)| r.as_str().map(|r| (g.clone(), r.to_string()))).collect())
                .unwrap_or_default(),
            default_role: sso.get("default_role").and_then(|d| d.as_str()).unwrap_or("member").to_string(),
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

/// The role a user's groups give them in `org` (the highest mapped role), else
/// the org's default; `None` = no membership.
pub fn role_for(org: &OrgSso, groups: &[String]) -> Option<String> {
    let rank = |r: &str| match r { "admin" => 3, "securityadmin" => 2, "member" => 1, _ => 0 };
    let mapped = org.role_map.iter()
        .filter(|(g, _)| groups.iter().any(|x| x == g || x.trim_start_matches('/') == g))
        .map(|(_, r)| r.as_str())
        .filter(|r| rank(r) > 0)
        .max_by_key(|r| rank(r));
    match mapped {
        Some(r) => Some(r.to_string()),
        None if rank(&org.default_role) > 0 => Some(org.default_role.clone()),
        None => None,
    }
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
}

#[derive(Deserialize)]
pub struct StartQuery {
    pub org: Option<String>,
    pub email: Option<String>,
    #[serde(rename = "return")]
    pub ret: Option<String>,
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
    let orgs = match platform_rows(&config, "orgs", None).await {
        Ok(rows) => orgs_with_sso(&rows),
        Err(e) => { warn!(error = %e, "sso: orgs unreadable"); return error(StatusCode::SERVICE_UNAVAILABLE, "Organisations are unavailable; try again."); }
    };
    let Some(org) = find_org(&orgs, q.org.as_deref(), q.email.as_deref()) else {
        return error(StatusCode::NOT_FOUND, "No organisation signs in that way. Check the organisation name or your work email.");
    };
    let pkce = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(pkce.as_bytes()));
    let state = SsoState {
        aud: STATE_AUD.into(),
        exp: chrono::Utc::now().timestamp() as usize + STATE_TTL_SECS,
        pkce,
        idp: org.idp_alias.clone(),
        ret: return_origin(&config, q.ret.as_deref()),
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
        .append_pair("code_challenge_method", "S256")
        .append_pair("kc_idp_hint", &org.idp_alias);
    if let Some(e) = q.email.as_deref().filter(|e| e.contains('@')) {
        url.query_pairs_mut().append_pair("login_hint", e);
    }
    info!(org = %org.handle, idp = %org.idp_alias, "sso: sign-in started");
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
    let back = |msg: &str| redirect(&format!("{}/#sso_error={}", state.ret, urlenc(msg)));
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
    if via != state.idp {
        warn!(started = %state.idp, via = %via, user = %sub, "sso: signed in through a different provider than the one asked for");
    }
    let mut org_id = "org-default".to_string();
    if let Some(org) = home {
        org_id = org.id.clone();
        let role = role_for(org, &groups);
        if let Err(e) = sync_membership(&config, org, &sub, &email, &name, role.as_deref()).await {
            warn!(error = %e, org = %org.handle, user = %sub, "sso: membership not updated");
            return back("Your organisation membership could not be updated. Try again.");
        }
        info!(org = %org.handle, user = %sub, email = %email, role = ?role, groups = ?groups, "sso: signed in");
    } else if !via.is_empty() {
        warn!(via = %via, user = %sub, "sso: provider belongs to no organisation");
    }

    let role = crate::auth_routes::pick_role(&realm_roles);
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
    fn only_orgs_with_a_provider_sign_in() {
        let o = orgs_with_sso(&rows());
        assert_eq!(o.iter().map(|x| x.handle.as_str()).collect::<Vec<_>>(), ["acme", "gamma"]);
        assert_eq!(find_org(&o, Some("acme"), None).unwrap().id, "org-acme");
        assert_eq!(find_org(&o, None, Some("Jane@ACME.test")).unwrap().id, "org-acme");
        assert!(find_org(&o, Some("beta"), None).is_none());
        assert!(find_org(&o, None, Some("x@other.test")).is_none());
    }

    #[test]
    fn groups_map_to_the_highest_role_else_the_default() {
        let o = orgs_with_sso(&rows());
        let acme = &o[0];
        assert_eq!(role_for(acme, &["a3t-admins".into(), "staff".into()]).as_deref(), Some("admin"));
        assert_eq!(role_for(acme, &["/a3t-sec".into()]).as_deref(), Some("securityadmin"));
        assert_eq!(role_for(acme, &["staff".into()]).as_deref(), Some("member"));
        assert_eq!(role_for(&o[1], &["staff".into()]), None, "default '' = no membership");
    }
}
