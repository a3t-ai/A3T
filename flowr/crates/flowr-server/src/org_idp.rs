//! An org connects its own identity provider, and keeps its people in step
//! (ID-10, .ai/A3T_ORG_SSO_SPEC.md §6 "self-service").
//!
//! **Its provider.** An org's admins (or security admins) register their OIDC
//! provider themselves: `PUT /security/orgs/{org}/idp`
//! `{issuer, client_id, client_secret, groups_claim?, domains?, display_name?}`.
//! FlowR reads the provider's discovery document (https only, never a private
//! address), then makes it an identity provider of realm `a3t` through the
//! Keycloak admin API, with what an org may not choose fixed: alias
//! `org-<handle>` (so its users can only ever land in this org, R2), groups
//! re-read at every sign-in (sync FORCE, R4), PKCE, no stored tokens, hidden
//! from the shared sign-in page, and one mapper: its groups claim → `a3t_groups`.
//! The org's sign-in (`settings.sso`) then names it. `GET` shows it (no secret),
//! `DELETE` disconnects it.
//!
//! **SCIM 2.0.** The org's provider pushes its people to
//! `/scim/v2/orgs/{org}/Users` with a bearer token the org's admins make at
//! `POST /security/orgs/{org}/scim/token` (shown once; a new one replaces it;
//! `DELETE` turns SCIM off). Someone deactivated (`active: false`) or deleted
//! there loses this org at once: their org membership and their roles on its
//! projects go, their sessions end, and sign-in through the org's provider is
//! refused while they stay deactivated. Every change is in the decision log.

use std::sync::Arc;

use axum::{
    extract::{Extension, Path, Query},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use flowr_ce::handlers::call_lancedb::{query_platform_table, sql_quote};
use flowr_policy::Principal;
use flowr_ce::flowr_storage::StorageBackend;

use crate::auth::AuthConfig;

pub const TOKENS: &str = "org_scim_tokens";
const TOKEN_COLUMNS: [&str; 6] = ["id", "org", "hash", "created_by", "created_at", "revoked"];
pub const USERS: &str = "org_scim_users";
const USER_COLUMNS: [&str; 10] = ["id", "org", "user_name", "external_id", "given_name", "family_name", "active", "created_at", "updated_at", "deprovisioned_at"];
const USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
const LIST_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
const PATCH_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";
const ERROR_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:Error";
const MASKED: &str = "**********";

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}
fn scim_err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, [("content-type", "application/scim+json")], Json(json!({ "schemas": [ERROR_SCHEMA], "status": code.as_u16().to_string(), "detail": msg.into() }))).into_response()
}
fn scim_ok(code: StatusCode, v: Value) -> Response {
    (code, [("content-type", "application/scim+json")], Json(v)).into_response()
}
fn col(r: &Value, k: &str) -> String {
    r.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}
fn who(p: Option<Extension<Principal>>) -> Principal {
    p.map(|Extension(p)| p).unwrap_or_else(|| Principal::service("anonymous"))
}
fn both(r: Result<Response, Response>) -> Response {
    r.unwrap_or_else(|e| e)
}

/// The Keycloak alias of an org's provider.
pub fn alias_of(org: &str) -> String {
    format!("org-{org}")
}

async fn ensure_tables(s: &dyn StorageBackend) -> Result<Arc<dyn flowr_ce::platform_db::PlatformDb>, String> {
    use flowr_ce::handlers::call_lancedb::{DEFAULT_PLATFORM_DB_PATH, PLATFORM_DB_RESOURCE};
    let db = flowr_ce::platform_db::open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    for (t, cols) in [(TOKENS, &TOKEN_COLUMNS[..]), (USERS, &USER_COLUMNS[..])] {
        let schema: Map<String, Value> = cols.iter().map(|c| (c.to_string(), json!("string"))).collect();
        db.create_table(t, Some(&schema), "create_if_not_exists").await?;
    }
    Ok(db)
}

// ── Its provider ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct IdpBody {
    issuer: String,
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    groups_claim: Option<String>,
    #[serde(default)]
    domains: Vec<String>,
    #[serde(default)]
    display_name: Option<String>,
}

/// A host an org may name: https, a public name or address — never the
/// platform's own network, loopback or cloud metadata.
async fn public_https(url: &str) -> Result<reqwest::Url, String> {
    let u = reqwest::Url::parse(url).map_err(|_| format!("{url:?} is not a URL"))?;
    if u.scheme() != "https" {
        return Err("the issuer must be https".into());
    }
    let host = u.host_str().ok_or("the issuer names no host")?.to_ascii_lowercase();
    if host == "localhost" || [".local", ".internal", ".svc", ".cluster.local", ".localdomain"].iter().any(|s| host.ends_with(s)) {
        return Err(format!("{host} is not a public host"));
    }
    let port = u.port_or_known_default().unwrap_or(443);
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), port)).await.map_err(|e| format!("{host}: {e}"))?.collect();
    if addrs.is_empty() || addrs.iter().any(|a| flowr_policy::egress::internal(&a.ip())) {
        return Err(format!("{host} resolves to a private address"));
    }
    Ok(u)
}

async fn discovery(client: &reqwest::Client, issuer: &str) -> Result<Value, String> {
    let issuer = issuer.trim_end_matches('/');
    let url = public_https(&format!("{issuer}/.well-known/openid-configuration")).await?;
    let r = client.get(url).timeout(std::time::Duration::from_secs(10)).send().await.map_err(|e| format!("the provider's discovery document: {e}"))?;
    if !r.status().is_success() {
        return Err(format!("the provider's discovery document answered {}", r.status()));
    }
    let d: Value = r.json().await.map_err(|e| format!("the provider's discovery document: {e}"))?;
    if d.get("issuer").and_then(|v| v.as_str()).map(|i| i.trim_end_matches('/')) != Some(issuer) {
        return Err("the discovery document names another issuer".into());
    }
    for k in ["authorization_endpoint", "token_endpoint", "jwks_uri"] {
        let v = d.get(k).and_then(|v| v.as_str()).ok_or_else(|| format!("the discovery document has no {k}"))?;
        public_https(v).await.map_err(|e| format!("{k}: {e}"))?;
    }
    Ok(d)
}

#[cfg(feature = "ee")]
async fn kc(idp: &crate::IdpAdminState) -> Result<(String, String), String> {
    use a3t_security::oidc_admin as k;
    let cfg = k::IdpAdminConfig::from_issuer(&idp.issuer, idp.admin_user.clone(), idp.admin_password.clone()).ok_or("no sign-in realm is configured")?;
    let token = k::admin_token(&idp.client, &cfg).await.map_err(|e| e.to_string())?;
    Ok((format!("{}/admin/realms/{}", cfg.base_url, cfg.realm), token))
}

#[cfg(feature = "ee")]
async fn kc_send(idp: &crate::IdpAdminState, token: &str, method: reqwest::Method, url: &str, body: Option<&Value>) -> Result<(u16, Value), String> {
    let mut rq = idp.client.request(method, url).bearer_auth(token);
    if let Some(b) = body {
        rq = rq.json(b);
    }
    let r = rq.send().await.map_err(|e| e.to_string())?;
    let code = r.status().as_u16();
    let text = r.text().await.unwrap_or_default();
    Ok((code, serde_json::from_str(&text).unwrap_or(Value::String(text))))
}

/// Once per realm, what every org's provider needs: the user profile keeps
/// `a3t_groups`, and the console's client puts it and the provider used in
/// its tokens (spec §3).
#[cfg(feature = "ee")]
async fn ensure_realm(idp: &crate::IdpAdminState, base: &str, token: &str) -> Result<(), String> {
    use reqwest::Method;
    let (c, mut profile) = kc_send(idp, token, Method::GET, &format!("{base}/users/profile"), None).await?;
    if c == 200 {
        let attrs = profile.get_mut("attributes").and_then(|a| a.as_array_mut()).ok_or("the realm's user profile has no attributes")?;
        if !attrs.iter().any(|a| a.get("name").and_then(|n| n.as_str()) == Some("a3t_groups")) {
            attrs.push(json!({ "name": "a3t_groups", "displayName": "A3T groups (from the org's provider)", "multivalued": true,
                "permissions": { "view": ["admin"], "edit": ["admin"] } }));
            let (c, b) = kc_send(idp, token, Method::PUT, &format!("{base}/users/profile"), Some(&profile)).await?;
            if c >= 300 { return Err(format!("the realm's user profile: {c} {b}")); }
        }
    }
    let client_id = std::env::var("FLOWR_SSO_CLIENT_ID").unwrap_or_else(|_| "a3t-ee".into());
    let (_, clients) = kc_send(idp, token, Method::GET, &format!("{base}/clients?clientId={client_id}"), None).await?;
    let Some(id) = clients.get(0).and_then(|c| c.get("id")).and_then(|v| v.as_str()).map(String::from) else {
        return Err(format!("the realm has no client {client_id}"));
    };
    let (_, mappers) = kc_send(idp, token, Method::GET, &format!("{base}/clients/{id}/protocol-mappers/models"), None).await?;
    let has = |n: &str| mappers.as_array().is_some_and(|a| a.iter().any(|m| m.get("name").and_then(|v| v.as_str()) == Some(n)));
    let wanted = [
        json!({ "name": "identity_provider", "protocol": "openid-connect", "protocolMapper": "oidc-usersessionmodel-note-mapper",
            "config": { "user.session.note": "identity_provider", "claim.name": "identity_provider", "jsonType.label": "String", "id.token.claim": "true", "access.token.claim": "true" } }),
        json!({ "name": "a3t_groups", "protocol": "openid-connect", "protocolMapper": "oidc-usermodel-attribute-mapper",
            "config": { "user.attribute": "a3t_groups", "claim.name": "a3t_groups", "jsonType.label": "String", "multivalued": "true", "id.token.claim": "true", "access.token.claim": "true" } }),
    ];
    for m in wanted {
        if !has(m["name"].as_str().unwrap_or_default()) {
            let (c, b) = kc_send(idp, token, Method::POST, &format!("{base}/clients/{id}/protocol-mappers/models"), Some(&m)).await?;
            if c >= 300 { return Err(format!("client mapper {}: {c} {b}", m["name"])); }
        }
    }
    Ok(())
}

async fn org_row(s: &dyn StorageBackend, org: &str) -> Result<Value, Response> {
    query_platform_table(s, "orgs", Some(format!("slug = {}", sql_quote(org))), 1).await.ok()
        .and_then(|r| r.into_iter().next()).ok_or_else(|| err(StatusCode::NOT_FOUND, format!("no org {org:?}")))
}

fn settings_of(row: &Value) -> Map<String, Value> {
    match row.get("settings") {
        Some(Value::String(t)) => serde_json::from_str(t).unwrap_or_default(),
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    }
}

async fn set_sso(s: &dyn StorageBackend, row: &Value, sso: Value) -> Result<(), String> {
    use flowr_ce::handlers::call_lancedb::{DEFAULT_PLATFORM_DB_PATH, PLATFORM_DB_RESOURCE};
    let mut settings = settings_of(row);
    settings.insert("sso".into(), sso);
    let mut v = Map::new();
    v.insert("settings".into(), json!(Value::Object(settings).to_string()));
    v.insert("updated_at".into(), json!(chrono::Utc::now().to_rfc3339()));
    let db = flowr_ce::platform_db::open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    db.update("orgs", &format!("id = {}", sql_quote(&col(row, "id"))), &v).await?;
    flowr_ce::access::invalidate();
    Ok(())
}

fn log(principal: &Principal, action: &str, org_id: &str, subject: &str, note: &str) {
    flowr_policy::audit::event_decision(principal, action, "Org", subject, None, Some(org_id), &flowr_policy::PolicyDecision::Allow, Some(note));
}

/// `PUT /security/orgs/:org/idp`
pub async fn put_idp(
    principal: Option<Extension<Principal>>,
    Extension(storage): Extension<Arc<dyn StorageBackend>>,
    Extension(idp): Extension<crate::IdpAdminState>,
    Path(org): Path<String>,
    Json(b): Json<IdpBody>,
) -> Response {
    both(async {
        let principal = who(principal);
        let s = storage.as_ref();
        flowr_ce::org_policy::allowed(s, &principal, &org).await?;
        let row = org_row(s, &org).await?;
        let bad = |m: String| err(StatusCode::BAD_REQUEST, m);
        if b.client_id.trim().is_empty() || b.client_id.len() > 200 {
            return Err(bad("client_id: the client A3T is registered as at your provider".into()));
        }
        let claim = b.groups_claim.clone().unwrap_or_else(|| "groups".into());
        if !claim.chars().all(|c| c.is_ascii_alphanumeric() || "_-.:/".contains(c)) || claim.len() > 100 {
            return Err(bad("groups_claim: a claim name".into()));
        }
        let domains: Vec<String> = b.domains.iter().map(|d| d.trim().to_ascii_lowercase()).filter(|d| !d.is_empty()).collect();
        if let Some(d) = domains.iter().find(|d| !d.contains('.') || !d.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')) {
            return Err(bad(format!("{d:?} is not an email domain")));
        }
        // A domain routes sign-ins: it may be claimed by one org only.
        let others = query_platform_table(s, "orgs", None, 10_000).await.unwrap_or_default();
        for o in crate::sso::orgs_with_sso(&others).into_iter().filter(|o| o.handle != org) {
            if let Some(d) = domains.iter().find(|d| o.domains.contains(d)) {
                return Err(err(StatusCode::CONFLICT, format!("{d} already signs in to another org")));
            }
        }
        let d = discovery(&idp.client, b.issuer.trim()).await.map_err(bad)?;
        let url = |k: &str| d.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
        #[cfg(feature = "ee")]
        {
            use reqwest::Method;
            let alias = alias_of(&org);
            let (base, token) = kc(&idp).await.map_err(|e| err(StatusCode::BAD_GATEWAY, e))?;
            ensure_realm(&idp, &base, &token).await.map_err(|e| err(StatusCode::BAD_GATEWAY, format!("the sign-in realm: {e}")))?;
            let inst = format!("{base}/identity-provider/instances/{alias}");
            let (exists, _) = kc_send(&idp, &token, Method::GET, &inst, None).await.map_err(|e| err(StatusCode::BAD_GATEWAY, e))?;
            let secret = match b.client_secret.as_deref().filter(|x| !x.is_empty()) {
                Some(x) => x.to_string(),
                None if exists == 200 => MASKED.to_string(), // Keycloak keeps the one it has
                None => return Err(bad("client_secret: needed the first time".into())),
            };
            let rep = json!({
                "alias": alias, "displayName": b.display_name.clone().unwrap_or_else(|| org.clone()), "providerId": "oidc", "enabled": true,
                "trustEmail": false, "storeToken": false, "addReadTokenRoleOnCreate": false, "linkOnly": false, "hideOnLogin": true,
                "firstBrokerLoginFlowAlias": "first broker login",
                "config": {
                    "issuer": url("issuer"), "authorizationUrl": url("authorization_endpoint"), "tokenUrl": url("token_endpoint"),
                    "jwksUrl": url("jwks_uri"), "userInfoUrl": url("userinfo_endpoint"), "logoutUrl": url("end_session_endpoint"),
                    "validateSignature": "true", "useJwksUrl": "true", "clientId": b.client_id.trim(), "clientSecret": secret,
                    "clientAuthMethod": "client_secret_post", "defaultScope": "openid email profile", "syncMode": "FORCE",
                    "pkceEnabled": "true", "pkceMethod": "S256", "hideOnLoginPage": "true", "backchannelSupported": "false",
                },
            });
            let (c, body) = if exists == 200 {
                kc_send(&idp, &token, Method::PUT, &inst, Some(&rep)).await
            } else {
                kc_send(&idp, &token, Method::POST, &format!("{base}/identity-provider/instances"), Some(&rep)).await
            }.map_err(|e| err(StatusCode::BAD_GATEWAY, e))?;
            if c >= 300 {
                return Err(err(StatusCode::BAD_GATEWAY, format!("the sign-in realm refused the provider: {c} {body}")));
            }
            // Its one mapper: the groups claim, re-read at every sign-in (R4).
            let (_, mappers) = kc_send(&idp, &token, Method::GET, &format!("{inst}/mappers"), None).await.map_err(|e| err(StatusCode::BAD_GATEWAY, e))?;
            for m in mappers.as_array().cloned().unwrap_or_default() {
                if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
                    let _ = kc_send(&idp, &token, Method::DELETE, &format!("{inst}/mappers/{id}"), None).await;
                }
            }
            let mapper = json!({ "name": "groups", "identityProviderAlias": alias, "identityProviderMapper": "oidc-user-attribute-idp-mapper",
                "config": { "syncMode": "FORCE", "claim": claim, "user.attribute": "a3t_groups" } });
            let (c, body) = kc_send(&idp, &token, Method::POST, &format!("{inst}/mappers"), Some(&mapper)).await.map_err(|e| err(StatusCode::BAD_GATEWAY, e))?;
            if c >= 300 {
                return Err(err(StatusCode::BAD_GATEWAY, format!("the groups mapper: {c} {body}")));
            }
            set_sso(s, &row, json!({ "idp_alias": alias, "domains": domains, "issuer": url("issuer"), "client_id": b.client_id.trim(),
                "groups_claim": claim, "connected_by": principal.id(), "connected_at": chrono::Utc::now().to_rfc3339() }))
                .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
            log(&principal, "org:idp_connect", &col(&row, "id"), &alias, &format!("{} · client {} · groups from {claim}", url("issuer"), b.client_id.trim()));
            info!(org = %org, alias = %alias, issuer = %url("issuer"), "org identity provider connected");
            return Ok(Json(json!({ "ok": true, "idp_alias": alias, "issuer": url("issuer"), "domains": domains, "groups_claim": claim })).into_response());
        }
        #[allow(unreachable_code)]
        { let _ = (url, row); Err(err(StatusCode::NOT_IMPLEMENTED, "connecting a provider needs the sign-in realm (EE build)")) }
    }.await)
}

/// `GET /security/orgs/:org/idp`
pub async fn get_idp(
    principal: Option<Extension<Principal>>,
    Extension(storage): Extension<Arc<dyn StorageBackend>>,
    Path(org): Path<String>,
) -> Response {
    both(async {
        let s = storage.as_ref();
        flowr_ce::org_policy::allowed(s, &who(principal), &org).await?;
        let row = org_row(s, &org).await?;
        let sso = settings_of(&row).get("sso").cloned().unwrap_or(Value::Null);
        let scim = query_platform_table(s, TOKENS, Some(format!("org = {} AND revoked = ''", sql_quote(&org))), 10).await.unwrap_or_default();
        let users = query_platform_table(s, USERS, Some(format!("org = {}", sql_quote(&org))), 100_000).await.unwrap_or_default();
        Ok(Json(json!({ "org": org, "sso": sso, "scim": {
            "on": !scim.is_empty(), "since": scim.first().map(|t| col(t, "created_at")), "base_url": format!("/scim/v2/orgs/{org}"),
            "users": users.len(), "deactivated": users.iter().filter(|u| col(u, "active") == "false").count(),
        } })).into_response())
    }.await)
}

/// `DELETE /security/orgs/:org/idp`
pub async fn delete_idp(
    principal: Option<Extension<Principal>>,
    Extension(storage): Extension<Arc<dyn StorageBackend>>,
    Extension(idp): Extension<crate::IdpAdminState>,
    Path(org): Path<String>,
) -> Response {
    both(async {
        let principal = who(principal);
        let s = storage.as_ref();
        flowr_ce::org_policy::allowed(s, &principal, &org).await?;
        let row = org_row(s, &org).await?;
        #[cfg(feature = "ee")]
        {
            let (base, token) = kc(&idp).await.map_err(|e| err(StatusCode::BAD_GATEWAY, e))?;
            let (c, b) = kc_send(&idp, &token, reqwest::Method::DELETE, &format!("{base}/identity-provider/instances/{}", alias_of(&org)), None)
                .await.map_err(|e| err(StatusCode::BAD_GATEWAY, e))?;
            if c >= 300 && c != 404 {
                return Err(err(StatusCode::BAD_GATEWAY, format!("the sign-in realm: {c} {b}")));
            }
        }
        let _ = &idp;
        set_sso(s, &row, Value::Null).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        log(&principal, "org:idp_disconnect", &col(&row, "id"), &alias_of(&org), "sign-in through the org's provider turned off");
        Ok(Json(json!({ "ok": true })).into_response())
    }.await)
}

// ── SCIM token ───────────────────────────────────────────────────────────

fn hash(t: &str) -> String {
    format!("{:x}", Sha256::digest(t.as_bytes()))
}

/// `POST /security/orgs/:org/scim/token` — a new token, shown once; the old one stops.
pub async fn new_token(
    principal: Option<Extension<Principal>>,
    Extension(storage): Extension<Arc<dyn StorageBackend>>,
    Path(org): Path<String>,
) -> Response {
    both(async {
        let principal = who(principal);
        let s = storage.as_ref();
        flowr_ce::org_policy::allowed(s, &principal, &org).await?;
        if principal.impersonated_by().is_some() {
            return Err(err(StatusCode::FORBIDDEN, "not while acting as someone else"));
        }
        let row = org_row(s, &org).await?;
        let db = ensure_tables(s).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let mut v = Map::new();
        v.insert("revoked".into(), json!(chrono::Utc::now().to_rfc3339()));
        let _ = db.update(TOKENS, &format!("org = {} AND revoked = ''", sql_quote(&org)), &v).await;
        let mut raw = [0u8; 32];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut raw).map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "no randomness"))?;
        let token = format!("a3ts_{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw));
        db.insert(TOKENS, &[json!({ "id": uuid::Uuid::now_v7().to_string(), "org": org, "hash": hash(&token), "created_by": principal.id(),
            "created_at": chrono::Utc::now().to_rfc3339(), "revoked": "" })]).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        log(&principal, "org:scim_token", &col(&row, "id"), &org, "a new SCIM token; the previous one stopped");
        Ok((StatusCode::CREATED, Json(json!({ "token": token, "base_url": format!("/scim/v2/orgs/{org}"),
            "note": "Shown once. Give it to your identity provider's SCIM provisioning." }))).into_response())
    }.await)
}

/// `DELETE /security/orgs/:org/scim/token` — SCIM off.
pub async fn drop_token(
    principal: Option<Extension<Principal>>,
    Extension(storage): Extension<Arc<dyn StorageBackend>>,
    Path(org): Path<String>,
) -> Response {
    both(async {
        let principal = who(principal);
        let s = storage.as_ref();
        flowr_ce::org_policy::allowed(s, &principal, &org).await?;
        let row = org_row(s, &org).await?;
        let db = ensure_tables(s).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let mut v = Map::new();
        v.insert("revoked".into(), json!(chrono::Utc::now().to_rfc3339()));
        db.update(TOKENS, &format!("org = {} AND revoked = ''", sql_quote(&org)), &v).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        log(&principal, "org:scim_off", &col(&row, "id"), &org, "SCIM turned off");
        Ok(Json(json!({ "ok": true })).into_response())
    }.await)
}

// ── SCIM 2.0 Users ───────────────────────────────────────────────────────

/// The org a SCIM request may act for: its bearer token is the org's live one.
async fn scim_caller(s: &dyn StorageBackend, org: &str, headers: &HeaderMap) -> Result<(Value, Principal), Response> {
    let t = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or_default().trim();
    let nope = || scim_err(StatusCode::UNAUTHORIZED, "a valid SCIM token for this org is needed");
    if !t.starts_with("a3ts_") {
        return Err(nope());
    }
    let rows = query_platform_table(s, TOKENS, Some(format!("org = {} AND hash = {} AND revoked = ''", sql_quote(org), sql_quote(&hash(t)))), 1).await.unwrap_or_default();
    if rows.is_empty() {
        return Err(nope());
    }
    let row = org_row(s, org).await.map_err(|_| nope())?;
    Ok((row, Principal::service(&format!("scim:{org}"))))
}

fn user_json(org: &str, u: &Value) -> Value {
    json!({
        "schemas": [USER_SCHEMA], "id": col(u, "id"), "externalId": col(u, "external_id"), "userName": col(u, "user_name"),
        "name": { "givenName": col(u, "given_name"), "familyName": col(u, "family_name") },
        "emails": [{ "value": col(u, "user_name"), "primary": true }],
        "active": col(u, "active") != "false",
        "meta": { "resourceType": "User", "created": col(u, "created_at"), "lastModified": col(u, "updated_at"),
            "location": format!("/scim/v2/orgs/{org}/Users/{}", col(u, "id")) },
    })
}

fn truthy(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) if s.eq_ignore_ascii_case("true") => Some(true),
        Value::String(s) if s.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

/// The fields a create or replace sets.
fn fields_of(b: &Value) -> Result<Map<String, Value>, Response> {
    let user = b.get("userName").and_then(|v| v.as_str()).map(|u| u.trim().to_ascii_lowercase())
        .filter(|u| !u.is_empty() && u.len() <= 254).ok_or_else(|| scim_err(StatusCode::BAD_REQUEST, "userName is required"))?;
    let mut m = Map::new();
    m.insert("user_name".into(), json!(user));
    m.insert("external_id".into(), json!(b.get("externalId").and_then(|v| v.as_str()).unwrap_or_default()));
    m.insert("given_name".into(), json!(b.pointer("/name/givenName").and_then(|v| v.as_str()).unwrap_or_default()));
    m.insert("family_name".into(), json!(b.pointer("/name/familyName").and_then(|v| v.as_str()).unwrap_or_default()));
    m.insert("active".into(), json!(if b.get("active").and_then(truthy).unwrap_or(true) { "true" } else { "false" }));
    Ok(m)
}

/// Someone the org's provider no longer lets in: they lose this org now.
async fn deprovision(s: &dyn StorageBackend, config: &AuthConfig, idp: &crate::IdpAdminState, org: &Value, email: &str, by: &Principal) -> Result<usize, String> {
    let org_id = col(org, "id");
    let slug = col(org, "slug");
    // Who they are here: the org's members with that email, and the realm's people with it.
    let mut users: Vec<String> = query_platform_table(s, "org_members", Some(format!("org_id = {}", sql_quote(&org_id))), 100_000).await?
        .into_iter().filter(|m| col(m, "email").eq_ignore_ascii_case(email)).map(|m| col(&m, "user_id")).collect();
    #[cfg(feature = "ee")]
    if let Ok((base, token)) = kc(idp).await {
        let enc: String = email.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
        if let Ok((200, found)) = kc_send(idp, &token, reqwest::Method::GET, &format!("{base}/users?email={enc}&exact=true"), None).await {
            users.extend(found.as_array().into_iter().flatten().filter_map(|u| u.get("id").and_then(|v| v.as_str()).map(String::from)));
        }
    }
    let _ = idp;
    users.sort();
    users.dedup();
    let db = config.platform.get().await;
    let mut removed = 0;
    for user in &users {
        // The org membership (invited or from sign-in alike).
        db.delete("org_members", &format!("org_id = {} AND user_id = {}", sql_quote(&org_id), sql_quote(user))).await?;
        // Their roles on the org's projects.
        for p in query_platform_table(s, "projects", Some(format!("owner_kind = 'org' AND owner = {}", sql_quote(&slug))), 10_000).await? {
            let members: Vec<Value> = serde_json::from_str(&col(&p, "members")).unwrap_or_default();
            let kept: Vec<Value> = members.iter().filter(|m| m.get("id").and_then(|v| v.as_str()) != Some(user.as_str())).cloned().collect();
            if kept.len() != members.len() {
                let mut v = Map::new();
                v.insert("members".into(), json!(serde_json::to_string(&kept).unwrap_or_default()));
                db.update("projects", &format!("namespace = {}", sql_quote(&col(&p, "namespace"))), &v).await?;
            }
        }
        // Their sessions end: tokens already out stop within seconds on every pod.
        for sess in query_platform_table(s, crate::sessions::TABLE, Some(format!("sub = {} AND revoked != '1'", sql_quote(user))), 1000).await.unwrap_or_default() {
            let _ = crate::sessions::revoke(config, &col(&sess, "sid"), &format!("removed by {slug}'s identity provider")).await;
        }
        removed += 1;
    }
    flowr_ce::access::invalidate();
    flowr_policy::audit::event_decision(by, "org:scim_deprovision", "User", email, None, Some(&org_id), &flowr_policy::PolicyDecision::Allow,
        Some(&format!("{} account(s) lost {slug}: membership, project roles and sessions", users.len())));
    warn!(org = %slug, accounts = users.len(), "scim: a person was deprovisioned");
    Ok(removed)
}

/// Whether the org's provider has turned this person off (sign-in refuses them).
pub async fn refused(config: &AuthConfig, org: &str, email: &str) -> bool {
    if email.is_empty() {
        return false;
    }
    let db = config.platform.get().await;
    let f = format!("org = {} AND user_name = {} AND active = 'false'", sql_quote(org), sql_quote(&email.to_ascii_lowercase()));
    db.query(USERS, Some(&f), 1).await.is_ok_and(|b| !flowr_ce::handlers::call_lancedb::batches_to_json(&b).is_empty())
}

#[derive(Deserialize)]
pub struct ListQ {
    #[serde(default)]
    filter: Option<String>,
    #[serde(default, rename = "startIndex")]
    start_index: Option<usize>,
    #[serde(default)]
    count: Option<usize>,
}

type Scim = (Extension<Arc<dyn StorageBackend>>, Extension<Arc<AuthConfig>>, Extension<crate::IdpAdminState>);

/// `GET /scim/v2/orgs/:org/ServiceProviderConfig`
pub async fn scim_config(Path(_org): Path<String>) -> Response {
    scim_ok(StatusCode::OK, json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"],
        "patch": { "supported": true }, "bulk": { "supported": false, "maxOperations": 0, "maxPayloadSize": 0 },
        "filter": { "supported": true, "maxResults": 200 }, "changePassword": { "supported": false },
        "sort": { "supported": false }, "etag": { "supported": false },
        "authenticationSchemes": [{ "type": "oauthbearertoken", "name": "Bearer token", "description": "The org's SCIM token" }],
    }))
}

/// `GET /scim/v2/orgs/:org/Users`
pub async fn list_users(
    (Extension(storage), _, _): Scim, headers: HeaderMap, Path(org): Path<String>, Query(q): Query<ListQ>,
) -> Response {
    both(async {
        let s = storage.as_ref();
        scim_caller(s, &org, &headers).await?;
        let mut filter = format!("org = {}", sql_quote(&org));
        if let Some(f) = q.filter.as_deref() {
            // The one filter providers send: userName eq "x" (or externalId).
            let re = regex::Regex::new(r#"(?i)^\s*(userName|externalId)\s+eq\s+"([^"]*)"\s*$"#).unwrap();
            let c = re.captures(f).ok_or_else(|| scim_err(StatusCode::BAD_REQUEST, "filter: userName eq \"…\" or externalId eq \"…\""))?;
            let (k, v) = if c[1].eq_ignore_ascii_case("userName") { ("user_name", c[2].to_ascii_lowercase()) } else { ("external_id", c[2].to_string()) };
            filter = format!("{filter} AND {k} = {}", sql_quote(&v));
        }
        let mut rows = query_platform_table(s, USERS, Some(filter), 100_000).await.unwrap_or_default();
        rows.sort_by_key(|r| col(r, "created_at"));
        let total = rows.len();
        let start = q.start_index.unwrap_or(1).max(1);
        let count = q.count.unwrap_or(100).min(200);
        let page: Vec<Value> = rows.iter().skip(start - 1).take(count).map(|u| user_json(&org, u)).collect();
        Ok(scim_ok(StatusCode::OK, json!({ "schemas": [LIST_SCHEMA], "totalResults": total, "startIndex": start, "itemsPerPage": page.len(), "Resources": page })))
    }.await)
}

async fn user_row(s: &dyn StorageBackend, org: &str, id: &str) -> Result<Value, Response> {
    query_platform_table(s, USERS, Some(format!("org = {} AND id = {}", sql_quote(org), sql_quote(id))), 1).await.ok()
        .and_then(|r| r.into_iter().next()).ok_or_else(|| scim_err(StatusCode::NOT_FOUND, format!("no user {id}")))
}

/// `GET /scim/v2/orgs/:org/Users/:id`
pub async fn get_user((Extension(storage), _, _): Scim, headers: HeaderMap, Path((org, id)): Path<(String, String)>) -> Response {
    both(async {
        let s = storage.as_ref();
        scim_caller(s, &org, &headers).await?;
        Ok(scim_ok(StatusCode::OK, user_json(&org, &user_row(s, &org, &id).await?)))
    }.await)
}

/// `POST /scim/v2/orgs/:org/Users`
pub async fn create_user(
    (Extension(storage), Extension(config), Extension(idp)): Scim, headers: HeaderMap, Path(org): Path<String>, Json(b): Json<Value>,
) -> Response {
    both(async {
        let s = storage.as_ref();
        let (orow, by) = scim_caller(s, &org, &headers).await?;
        let mut f = fields_of(&b)?;
        let user = col(&Value::Object(f.clone()), "user_name");
        if !query_platform_table(s, USERS, Some(format!("org = {} AND user_name = {}", sql_quote(&org), sql_quote(&user))), 1).await.unwrap_or_default().is_empty() {
            return Err(scim_err(StatusCode::CONFLICT, format!("{user} is already provisioned")));
        }
        let now = chrono::Utc::now().to_rfc3339();
        let id = uuid::Uuid::now_v7().to_string();
        f.insert("id".into(), json!(id));
        f.insert("org".into(), json!(org));
        f.insert("created_at".into(), json!(now));
        f.insert("updated_at".into(), json!(now));
        f.insert("deprovisioned_at".into(), json!(""));
        let db = ensure_tables(s).await.map_err(|e| scim_err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        db.insert(USERS, &[Value::Object(f.clone())]).await.map_err(|e| scim_err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        if col(&Value::Object(f.clone()), "active") == "false" {
            deprovision(s, &config, &idp, &orow, &user, &by).await.map_err(|e| scim_err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        }
        log(&by, "org:scim_provision", &col(&orow, "id"), &user, "provisioned by the org's identity provider");
        Ok(scim_ok(StatusCode::CREATED, user_json(&org, &Value::Object(f))))
    }.await)
}

/// Apply new fields to a user; turning them off deprovisions them.
async fn change(s: &dyn StorageBackend, config: &AuthConfig, idp: &crate::IdpAdminState, org: &str, orow: &Value, by: &Principal, row: &Value, next: Map<String, Value>) -> Result<Value, Response> {
    let was_active = col(row, "active") != "false";
    let mut v = next.clone();
    v.insert("updated_at".into(), json!(chrono::Utc::now().to_rfc3339()));
    let now_active = v.get("active").and_then(|x| x.as_str()).map_or(was_active, |a| a != "false");
    if was_active && !now_active {
        v.insert("deprovisioned_at".into(), json!(chrono::Utc::now().to_rfc3339()));
    }
    let db = ensure_tables(s).await.map_err(|e| scim_err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    db.update(USERS, &format!("id = {}", sql_quote(&col(row, "id"))), &v).await.map_err(|e| scim_err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let user = v.get("user_name").and_then(|x| x.as_str()).map(String::from).unwrap_or_else(|| col(row, "user_name"));
    if was_active && !now_active {
        deprovision(s, config, idp, orow, &user, by).await.map_err(|e| scim_err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    } else if !was_active && now_active {
        log(by, "org:scim_reactivate", &col(orow, "id"), &user, "may sign in again; their role comes back from their groups at sign-in");
    }
    let mut merged = row.as_object().cloned().unwrap_or_default();
    merged.extend(v);
    let _ = org;
    Ok(Value::Object(merged))
}

/// `PUT /scim/v2/orgs/:org/Users/:id`
pub async fn replace_user(
    (Extension(storage), Extension(config), Extension(idp)): Scim, headers: HeaderMap, Path((org, id)): Path<(String, String)>, Json(b): Json<Value>,
) -> Response {
    both(async {
        let s = storage.as_ref();
        let (orow, by) = scim_caller(s, &org, &headers).await?;
        let row = user_row(s, &org, &id).await?;
        let f = fields_of(&b)?;
        let u = change(s, &config, &idp, &org, &orow, &by, &row, f).await?;
        Ok(scim_ok(StatusCode::OK, user_json(&org, &u)))
    }.await)
}

/// `PATCH /scim/v2/orgs/:org/Users/:id` — `replace` of `active` (and names), as
/// Entra ID, Okta and others send it.
pub async fn patch_user(
    (Extension(storage), Extension(config), Extension(idp)): Scim, headers: HeaderMap, Path((org, id)): Path<(String, String)>, Json(b): Json<Value>,
) -> Response {
    both(async {
        let s = storage.as_ref();
        let (orow, by) = scim_caller(s, &org, &headers).await?;
        let row = user_row(s, &org, &id).await?;
        if !b.get("schemas").and_then(|v| v.as_array()).is_some_and(|a| a.iter().any(|x| x == PATCH_SCHEMA)) {
            return Err(scim_err(StatusCode::BAD_REQUEST, "a PatchOp"));
        }
        let mut next = Map::new();
        let mut set = |path: &str, v: &Value| {
            match path.to_ascii_lowercase().as_str() {
                "active" => if let Some(a) = truthy(v) { next.insert("active".into(), json!(if a { "true" } else { "false" })); },
                "username" => if let Some(u) = v.as_str() { next.insert("user_name".into(), json!(u.trim().to_ascii_lowercase())); },
                "externalid" => if let Some(u) = v.as_str() { next.insert("external_id".into(), json!(u)); },
                "name.givenname" => if let Some(u) = v.as_str() { next.insert("given_name".into(), json!(u)); },
                "name.familyname" => if let Some(u) = v.as_str() { next.insert("family_name".into(), json!(u)); },
                _ => {}
            }
        };
        for op in b.get("Operations").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
            let kind = op.get("op").and_then(|v| v.as_str()).unwrap_or_default().to_ascii_lowercase();
            if kind != "replace" && kind != "add" {
                continue;
            }
            let value = op.get("value").cloned().unwrap_or(Value::Null);
            match op.get("path").and_then(|v| v.as_str()) {
                Some(p) => set(p, &value),
                None => for (k, v) in value.as_object().cloned().unwrap_or_default() {
                    if let (Some(o), "name") = (v.as_object(), k.as_str()) {
                        for (nk, nv) in o { set(&format!("name.{nk}"), nv); }
                    } else {
                        set(&k, &v);
                    }
                },
            }
        }
        let u = change(s, &config, &idp, &org, &orow, &by, &row, next).await?;
        Ok(scim_ok(StatusCode::OK, user_json(&org, &u)))
    }.await)
}

/// `DELETE /scim/v2/orgs/:org/Users/:id` — removed at the provider: gone here too.
pub async fn delete_user((Extension(storage), Extension(config), Extension(idp)): Scim, headers: HeaderMap, Path((org, id)): Path<(String, String)>) -> Response {
    both(async {
        let s = storage.as_ref();
        let (orow, by) = scim_caller(s, &org, &headers).await?;
        let row = user_row(s, &org, &id).await?;
        let user = col(&row, "user_name");
        deprovision(s, &config, &idp, &orow, &user, &by).await.map_err(|e| scim_err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        // Kept as deactivated, so sign-in still refuses them.
        let mut v = Map::new();
        v.insert("active".into(), json!("false"));
        v.insert("deprovisioned_at".into(), json!(chrono::Utc::now().to_rfc3339()));
        v.insert("updated_at".into(), json!(chrono::Utc::now().to_rfc3339()));
        let db = ensure_tables(s).await.map_err(|e| scim_err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        db.update(USERS, &format!("id = {}", sql_quote(&id)), &v).await.map_err(|e| scim_err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        Ok(StatusCode::NO_CONTENT.into_response())
    }.await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_and_truthiness() {
        let f = fields_of(&json!({ "userName": " Jane@Acme.COM ", "active": "False", "name": { "givenName": "Jane" } })).unwrap();
        assert_eq!(f["user_name"], "jane@acme.com");
        assert_eq!(f["active"], "false");
        assert_eq!(f["given_name"], "Jane");
        assert!(fields_of(&json!({ "active": true })).is_err());
        assert_eq!(truthy(&json!("TRUE")), Some(true));
        assert_eq!(truthy(&json!(1)), None);
    }

    #[tokio::test]
    async fn an_org_may_not_name_a_private_issuer() {
        for u in ["http://idp.example.com", "https://localhost/realms/x", "https://keycloak.a3t.svc/realms/x", "https://10.0.0.5/x", "https://169.254.169.254/x"] {
            assert!(public_https(u).await.is_err(), "{u}");
        }
    }
}
