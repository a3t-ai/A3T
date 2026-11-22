//! Keycloak Admin API client — the "manage the IdP from A3T" surface: obtain an
//! admin token, create users, assign realm roles, list users. This is the
//! EE feature behind "connect to Keycloak + create users from the A3T UI".
//!
//! Dev/UAT authenticates as the **master-realm admin** via the `admin-cli`
//! password grant, so no Keycloak reconfiguration is required. **Production**
//! should instead provision a scoped confidential client (service account +
//! `realm-management` `manage-users`) and use `client_credentials` — only
//! [`admin_token`] would change; the user/role calls are identical.
//!
//! Like [`crate::oidc`], this is transport machinery only — it knows nothing
//! about the consumer's `Principal`/roles; the caller maps and gates.

use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum IdpAdminError {
    #[error("admin auth failed: {0}")]
    Auth(String),
    #[error("keycloak api error {status}: {body}")]
    Api { status: u16, body: String },
    #[error("http error: {0}")]
    Http(String),
    #[error("not configured: {0}")]
    NotConfigured(String),
    /// The password was right, but the account has a required action pending
    /// (e.g. a temporary password to replace), so Keycloak issues no token.
    #[error("account not fully set up")]
    SetupRequired,
}

/// Connection + credentials for the Keycloak Admin API.
#[derive(Debug, Clone)]
pub struct IdpAdminConfig {
    /// Keycloak base URL, e.g. `http://keycloak.a3t.svc:8080` (no `/realms/...`).
    pub base_url: String,
    /// The realm whose users we manage, e.g. `a3t`.
    pub realm: String,
    /// Master-realm admin username (admin-cli password grant).
    pub admin_user: String,
    /// Master-realm admin password.
    pub admin_password: String,
}

impl IdpAdminConfig {
    /// Derive `base_url` + `realm` from an OIDC issuer like
    /// `http://keycloak.a3t.svc:8080/realms/a3t`. Returns `None` if the issuer
    /// isn't a realm URL (so the caller can report "not configured").
    pub fn from_issuer(issuer: &str, admin_user: String, admin_password: String) -> Option<Self> {
        let issuer = issuer.trim_end_matches('/');
        let idx = issuer.find("/realms/")?;
        let base_url = issuer[..idx].to_string();
        let realm = issuer[idx + "/realms/".len()..].to_string();
        if base_url.is_empty() || realm.is_empty() || admin_password.is_empty() {
            return None;
        }
        Some(Self { base_url, realm, admin_user, admin_password })
    }
}

/// A user to create in the realm.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewUser {
    pub username: String,
    #[serde(default)]
    pub email: Option<String>,
    pub password: String,
    /// Full name (e.g. "Carol Diaz"), split into first/last. Optional — if empty,
    /// the username is used. Keycloak's VERIFY_PROFILE treats a user with no
    /// first/last name as "not fully set up" and refuses login, so we always
    /// send a non-empty first AND last name.
    #[serde(default)]
    pub name: Option<String>,
    /// If true, the user must reset the password on first login.
    #[serde(default)]
    pub temporary: bool,
    /// Realm roles to assign (e.g. `operator`, `admin`). Unknown roles are skipped.
    #[serde(default)]
    pub roles: Vec<String>,
}

impl NewUser {
    /// (firstName, lastName), both guaranteed non-empty so the Keycloak profile is
    /// complete and the user can log in immediately.
    fn names(&self) -> (String, String) {
        let full = self.name.as_deref().unwrap_or("").trim();
        if full.is_empty() {
            return (self.username.clone(), self.username.clone());
        }
        match full.split_once(char::is_whitespace) {
            Some((first, rest)) => {
                let last = rest.trim();
                (first.to_string(), if last.is_empty() { self.username.clone() } else { last.to_string() })
            }
            None => (full.to_string(), self.username.clone()),
        }
    }
}

/// Obtain an admin access token via the master realm `admin-cli` password grant.
pub async fn admin_token(
    client: &reqwest::Client,
    cfg: &IdpAdminConfig,
) -> Result<String, IdpAdminError> {
    let url = format!("{}/realms/master/protocol/openid-connect/token", cfg.base_url);
    let resp = client
        .post(&url)
        .form(&[
            ("grant_type", "password"),
            ("client_id", "admin-cli"),
            ("username", cfg.admin_user.as_str()),
            ("password", cfg.admin_password.as_str()),
        ])
        .send()
        .await
        .map_err(|e| IdpAdminError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(IdpAdminError::Auth(format!("token endpoint returned {}", resp.status())));
    }
    let v: Value = resp.json().await.map_err(|e| IdpAdminError::Http(e.to_string()))?;
    v.get("access_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| IdpAdminError::Auth("no access_token in response".into()))
}

/// Create a user, set its password, and assign realm roles. Returns the new id.
pub async fn create_user(
    client: &reqwest::Client,
    cfg: &IdpAdminConfig,
    token: &str,
    user: &NewUser,
) -> Result<String, IdpAdminError> {
    let url = format!("{}/admin/realms/{}/users", cfg.base_url, cfg.realm);
    let (first_name, last_name) = user.names();
    let body = json!({
        "username": user.username,
        "email": user.email,
        "firstName": first_name,
        "lastName": last_name,
        "enabled": true,
        "emailVerified": true,
        // Clear any realm-default required actions (VERIFY_EMAIL, UPDATE_PASSWORD,
        // …). Combined with a complete profile (first/last name above), this makes
        // a freshly provisioned user immediately loginable — otherwise Keycloak
        // reports "Account is not fully set up" and refuses the password grant.
        // `temporary: true` still forces a reset at first login when requested.
        "requiredActions": [],
        "credentials": [{ "type": "password", "value": user.password, "temporary": user.temporary }],
    });
    let resp = client
        .post(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| IdpAdminError::Http(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(IdpAdminError::Api { status: status.as_u16(), body });
    }
    // Keycloak returns the new id in the Location header: .../users/{id}
    let id = resp
        .headers()
        .get("location")
        .and_then(|h| h.to_str().ok())
        .and_then(|l| l.rsplit('/').next())
        .map(|s| s.to_string())
        .ok_or_else(|| IdpAdminError::Api { status: status.as_u16(), body: "no Location header on created user".into() })?;
    if !user.roles.is_empty() {
        assign_realm_roles(client, cfg, token, &id, &user.roles).await?;
    }
    Ok(id)
}

/// Assign realm-level roles to a user (resolves each role's representation first;
/// unknown role names are skipped rather than failing the whole call).
pub async fn assign_realm_roles(
    client: &reqwest::Client,
    cfg: &IdpAdminConfig,
    token: &str,
    user_id: &str,
    roles: &[String],
) -> Result<(), IdpAdminError> {
    let mut role_reps = Vec::new();
    for role in roles {
        let url = format!("{}/admin/realms/{}/roles/{}", cfg.base_url, cfg.realm, role);
        let resp = client
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| IdpAdminError::Http(e.to_string()))?;
        if resp.status().is_success() {
            if let Ok(v) = resp.json::<Value>().await {
                role_reps.push(v);
            }
        }
    }
    if role_reps.is_empty() {
        return Ok(());
    }
    let url = format!("{}/admin/realms/{}/users/{}/role-mappings/realm", cfg.base_url, cfg.realm, user_id);
    let resp = client
        .post(&url)
        .bearer_auth(token)
        .json(&role_reps)
        .send()
        .await
        .map_err(|e| IdpAdminError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(IdpAdminError::Api { status, body });
    }
    Ok(())
}

/// Fields to update on an existing user. All optional — only present fields are
/// changed. `roles: Some(..)` REPLACES the user's manageable realm roles (Keycloak
/// default composite roles like `default-roles-*` are preserved).
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct UpdateUser {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    /// New password (empty/omitted = unchanged).
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub temporary: bool,
    #[serde(default)]
    pub roles: Option<Vec<String>>,
}

/// A user's identity resolved from a successful password login.
#[derive(Debug, Clone)]
pub struct UserIdentity {
    pub sub: String,
    pub username: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub roles: Vec<String>,
    /// The realm says (`amr`) a second factor was used.
    pub second_factor: bool,
}

/// Authenticate an END USER by username/password against Keycloak (direct grant,
/// public client). Used by the console's unified login: `/auth/login` tries the
/// local user store first, then falls back to this. Returns the user's identity
/// (decoded from the issued access token — trusted because it came straight from
/// the issuer over TLS in this request). NOT for admin ops (see [`admin_token`]).
pub async fn password_login(
    client: &reqwest::Client,
    base_url: &str,
    realm: &str,
    client_id: &str,
    username: &str,
    password: &str,
    otp: Option<&str>,
) -> Result<UserIdentity, IdpAdminError> {
    let url = format!("{}/realms/{}/protocol/openid-connect/token", base_url.trim_end_matches('/'), realm);
    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "password"),
        ("client_id", client_id),
        ("username", username),
        ("password", password),
        ("scope", "openid"),
    ];
    // ID-9: the authenticator code, for accounts whose sign-in needs one.
    if let Some(code) = otp.map(str::trim).filter(|c| !c.is_empty()) {
        form.push(("otp", code));
    }
    let resp = client
        .post(&url)
        .form(&form)
        .send()
        .await
        .map_err(|e| IdpAdminError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status();
        // Keycloak checks the password before required actions: "Account is not
        // fully set up" means the password was right and an action is pending.
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        let why = body.get("error_description").and_then(|d| d.as_str()).unwrap_or_default();
        if status.as_u16() == 400 && why.contains("not fully set up") {
            return Err(IdpAdminError::SetupRequired);
        }
        return Err(IdpAdminError::Auth(format!("password grant returned {status}")));
    }
    let v: Value = resp.json().await.map_err(|e| IdpAdminError::Http(e.to_string()))?;
    let token = v.get("access_token").and_then(|t| t.as_str())
        .ok_or_else(|| IdpAdminError::Auth("no access_token in response".into()))?;
    let payload_b64 = token.split('.').nth(1)
        .ok_or_else(|| IdpAdminError::Auth("malformed access token".into()))?;
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|e| IdpAdminError::Auth(format!("token decode: {e}")))?;
    let claims: Value = serde_json::from_slice(&bytes).map_err(|e| IdpAdminError::Http(e.to_string()))?;
    let roles = claims.get("realm_access").and_then(|r| r.get("roles")).and_then(|r| r.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    Ok(UserIdentity {
        sub: claims.get("sub").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        username: claims.get("preferred_username").and_then(|v| v.as_str()).unwrap_or(username).to_string(),
        email: claims.get("email").and_then(|v| v.as_str()).map(String::from),
        name: claims.get("name").and_then(|v| v.as_str()).map(String::from),
        roles,
        second_factor: claims.get("amr").and_then(|v| v.as_array()).is_some_and(|a| a.iter().any(|m| matches!(m.as_str(), Some("otp" | "mfa" | "totp" | "hwk" | "swk")))),
    })
}

/// Whether a user has an authenticator (one-time code) set up. A password
/// grant that carried a code proves a second factor only for such a user: the
/// realm ignores a code from anyone without one.
pub async fn has_otp(client: &reqwest::Client, cfg: &IdpAdminConfig, token: &str, user_id: &str) -> Result<bool, IdpAdminError> {
    let url = format!("{}/admin/realms/{}/users/{}/credentials", cfg.base_url, cfg.realm, user_id);
    let resp = client.get(&url).bearer_auth(token).send().await.map_err(|e| IdpAdminError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(IdpAdminError::Api { status, body: resp.text().await.unwrap_or_default() });
    }
    let v: Value = resp.json().await.map_err(|e| IdpAdminError::Http(e.to_string()))?;
    Ok(v.as_array().is_some_and(|a| a.iter().any(|c| c.get("type").and_then(|t| t.as_str()) == Some("otp"))))
}

/// Fetch a single user's representation.
pub async fn get_user(
    client: &reqwest::Client,
    cfg: &IdpAdminConfig,
    token: &str,
    user_id: &str,
) -> Result<Value, IdpAdminError> {
    let url = format!("{}/admin/realms/{}/users/{}", cfg.base_url, cfg.realm, user_id);
    let resp = client.get(&url).bearer_auth(token).send().await.map_err(|e| IdpAdminError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(IdpAdminError::Api { status, body: resp.text().await.unwrap_or_default() });
    }
    resp.json::<Value>().await.map_err(|e| IdpAdminError::Http(e.to_string()))
}

/// The realm role names directly assigned to a user.
pub async fn user_realm_roles(
    client: &reqwest::Client,
    cfg: &IdpAdminConfig,
    token: &str,
    user_id: &str,
) -> Result<Vec<String>, IdpAdminError> {
    let url = format!("{}/admin/realms/{}/users/{}/role-mappings/realm", cfg.base_url, cfg.realm, user_id);
    let resp = client.get(&url).bearer_auth(token).send().await.map_err(|e| IdpAdminError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(IdpAdminError::Api { status, body: resp.text().await.unwrap_or_default() });
    }
    let arr = resp.json::<Vec<Value>>().await.map_err(|e| IdpAdminError::Http(e.to_string()))?;
    Ok(arr.iter().filter_map(|r| r.get("name").and_then(|v| v.as_str()).map(String::from)).collect())
}

/// Remove realm roles from a user (resolves each role rep, then DELETEs the mapping).
pub async fn remove_realm_roles(
    client: &reqwest::Client,
    cfg: &IdpAdminConfig,
    token: &str,
    user_id: &str,
    roles: &[String],
) -> Result<(), IdpAdminError> {
    let mut role_reps = Vec::new();
    for role in roles {
        let url = format!("{}/admin/realms/{}/roles/{}", cfg.base_url, cfg.realm, role);
        if let Ok(resp) = client.get(&url).bearer_auth(token).send().await {
            if resp.status().is_success() {
                if let Ok(v) = resp.json::<Value>().await { role_reps.push(v); }
            }
        }
    }
    if role_reps.is_empty() { return Ok(()); }
    let url = format!("{}/admin/realms/{}/users/{}/role-mappings/realm", cfg.base_url, cfg.realm, user_id);
    let resp = client.request(reqwest::Method::DELETE, &url).bearer_auth(token).json(&role_reps).send().await
        .map_err(|e| IdpAdminError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(IdpAdminError::Api { status, body: resp.text().await.unwrap_or_default() });
    }
    Ok(())
}

/// Update a user: profile fields, optional password reset, and (if `roles` is
/// given) reconcile realm roles to exactly that set (preserving `default-roles-*`).
pub async fn update_user(
    client: &reqwest::Client,
    cfg: &IdpAdminConfig,
    token: &str,
    user_id: &str,
    upd: &UpdateUser,
) -> Result<(), IdpAdminError> {
    // 1. Profile PUT (merge onto the current representation so we don't clobber).
    let mut rep = get_user(client, cfg, token, user_id).await?;
    if let Some(obj) = rep.as_object_mut() {
        if let Some(email) = &upd.email { obj.insert("email".into(), json!(email)); }
        if let Some(enabled) = upd.enabled { obj.insert("enabled".into(), json!(enabled)); }
        if let Some(name) = &upd.name {
            let full = name.trim();
            if !full.is_empty() {
                let (first, last) = match full.split_once(char::is_whitespace) {
                    Some((f, r)) => (f.to_string(), { let l = r.trim(); if l.is_empty() { f.to_string() } else { l.to_string() } }),
                    None => (full.to_string(), full.to_string()),
                };
                obj.insert("firstName".into(), json!(first));
                obj.insert("lastName".into(), json!(last));
            }
        }
    }
    let url = format!("{}/admin/realms/{}/users/{}", cfg.base_url, cfg.realm, user_id);
    let resp = client.put(&url).bearer_auth(token).json(&rep).send().await.map_err(|e| IdpAdminError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(IdpAdminError::Api { status, body: resp.text().await.unwrap_or_default() });
    }
    // 2. Optional password reset.
    if let Some(pw) = &upd.password {
        if !pw.is_empty() {
            let url = format!("{}/admin/realms/{}/users/{}/reset-password", cfg.base_url, cfg.realm, user_id);
            let body = json!({ "type": "password", "value": pw, "temporary": upd.temporary });
            let resp = client.put(&url).bearer_auth(token).json(&body).send().await.map_err(|e| IdpAdminError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                return Err(IdpAdminError::Api { status, body: resp.text().await.unwrap_or_default() });
            }
        }
    }
    // 3. Optional role reconcile (preserve Keycloak default composite roles).
    if let Some(desired) = &upd.roles {
        let current = user_realm_roles(client, cfg, token, user_id).await.unwrap_or_default();
        let to_add: Vec<String> = desired.iter().filter(|r| !current.contains(r)).cloned().collect();
        let to_remove: Vec<String> = current.iter()
            .filter(|r| !desired.contains(r) && !r.starts_with("default-roles"))
            .cloned().collect();
        if !to_add.is_empty() { assign_realm_roles(client, cfg, token, user_id, &to_add).await?; }
        if !to_remove.is_empty() { remove_realm_roles(client, cfg, token, user_id, &to_remove).await?; }
    }
    Ok(())
}

/// Delete a user by id.
pub async fn delete_user(
    client: &reqwest::Client,
    cfg: &IdpAdminConfig,
    token: &str,
    user_id: &str,
) -> Result<(), IdpAdminError> {
    let url = format!("{}/admin/realms/{}/users/{}", cfg.base_url, cfg.realm, user_id);
    let resp = client.delete(&url).bearer_auth(token).send().await.map_err(|e| IdpAdminError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(IdpAdminError::Api { status, body: resp.text().await.unwrap_or_default() });
    }
    Ok(())
}

/// List realm users (raw Keycloak user representations; the caller projects the
/// fields it wants for the UI).
pub async fn list_users(
    client: &reqwest::Client,
    cfg: &IdpAdminConfig,
    token: &str,
) -> Result<Vec<Value>, IdpAdminError> {
    let url = format!("{}/admin/realms/{}/users?max=200", cfg.base_url, cfg.realm);
    let resp = client
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| IdpAdminError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(IdpAdminError::Api { status, body });
    }
    resp.json::<Vec<Value>>().await.map_err(|e| IdpAdminError::Http(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_derives_base_and_realm_from_issuer() {
        let cfg = IdpAdminConfig::from_issuer(
            "http://keycloak.a3t.svc:8080/realms/a3t",
            "admin".into(),
            "pw".into(),
        )
        .unwrap();
        assert_eq!(cfg.base_url, "http://keycloak.a3t.svc:8080");
        assert_eq!(cfg.realm, "a3t");
    }

    #[test]
    fn config_none_without_password_or_realm() {
        // no password → not configured
        assert!(IdpAdminConfig::from_issuer("http://kc/realms/a3t", "admin".into(), "".into()).is_none());
        // not a realm issuer
        assert!(IdpAdminConfig::from_issuer("http://kc/", "admin".into(), "pw".into()).is_none());
    }
}
