//! OIDC / RS256 token verification against a JWKS endpoint (E2 #4).
//!
//! This is the reusable AuthN machinery — fetch + cache an issuer's JWKS, verify
//! a bearer token's RS256 signature, validate `iss`/`aud`/`exp` — and nothing
//! else. It never touches authorization (that's the choke point) and it doesn't
//! know the consumer's `Principal` type: `verify` returns the validated claims,
//! and the consumer maps them onto whatever identity it uses (A3T EE maps
//! `sub`/`realm_access.roles` → `flowr_policy::Principal` via `roles::canonicalize`).

use std::collections::HashMap;
use std::sync::RwLock;

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum AuthnError {
    #[error("no bearer token")]
    MissingToken,
    #[error("malformed token: {0}")]
    Malformed(String),
    #[error("token has no `kid`")]
    NoKid,
    #[error("unknown signing key `{0}`")]
    UnknownKey(String),
    #[error("signature or claims invalid: {0}")]
    Invalid(String),
    #[error("jwks fetch failed: {0}")]
    Jwks(String),
}

/// Issuer + audience the layer validates against, plus where to fetch keys.
#[derive(Debug, Clone)]
pub struct AuthnConfig {
    pub issuer: String,
    pub audience: Option<String>,
    pub jwks_uri: String,
}

impl AuthnConfig {
    /// Standard OIDC issuer → its Keycloak-style JWKS URL. (For non-Keycloak
    /// issuers, set `jwks_uri` from the discovery document instead.)
    pub fn from_issuer(issuer: impl Into<String>, audience: Option<String>) -> Self {
        let issuer = issuer.into();
        let jwks_uri = format!(
            "{}/protocol/openid-connect/certs",
            issuer.trim_end_matches('/')
        );
        Self { issuer, audience, jwks_uri }
    }
}

/// Caches an issuer's JWKS keyed by `kid`, refreshing on an unknown `kid` so key
/// rotation is picked up without a fixed TTL.
pub struct JwksCache {
    jwks_uri: String,
    keys: RwLock<HashMap<String, DecodingKey>>,
    client: reqwest::Client,
}

impl JwksCache {
    pub fn new(jwks_uri: impl Into<String>) -> Self {
        Self {
            jwks_uri: jwks_uri.into(),
            keys: RwLock::new(HashMap::new()),
            client: reqwest::Client::new(),
        }
    }

    /// Test seam: preload keys without a network fetch.
    #[doc(hidden)]
    pub fn from_keys(keys: HashMap<String, DecodingKey>) -> Self {
        Self {
            jwks_uri: String::new(),
            keys: RwLock::new(keys),
            client: reqwest::Client::new(),
        }
    }

    async fn refresh(&self) -> Result<(), AuthnError> {
        let jwks: JwkSet = self
            .client
            .get(&self.jwks_uri)
            .send()
            .await
            .map_err(|e| AuthnError::Jwks(e.to_string()))?
            .json()
            .await
            .map_err(|e| AuthnError::Jwks(e.to_string()))?;
        let mut map = HashMap::new();
        for jwk in &jwks.keys {
            if let Some(kid) = jwk.common.key_id.clone() {
                if let Ok(dk) = DecodingKey::from_jwk(jwk) {
                    map.insert(kid, dk);
                }
            }
        }
        *self.keys.write().unwrap() = map;
        Ok(())
    }

    async fn key_for(&self, kid: &str) -> Result<DecodingKey, AuthnError> {
        if let Some(k) = self.keys.read().unwrap().get(kid).cloned() {
            return Ok(k);
        }
        self.refresh().await?;
        self.keys
            .read()
            .unwrap()
            .get(kid)
            .cloned()
            .ok_or_else(|| AuthnError::UnknownKey(kid.to_string()))
    }
}

/// Verify a bearer token: RS256 signature against the issuer's JWKS, plus
/// `iss`/`aud`/`exp` validation. Returns the validated claims (JSON) for the
/// consumer to map onto its `Principal`.
pub async fn verify(
    token: &str,
    cache: &JwksCache,
    cfg: &AuthnConfig,
) -> Result<Value, AuthnError> {
    let header = decode_header(token).map_err(|e| AuthnError::Malformed(e.to_string()))?;
    if header.alg != Algorithm::RS256 {
        return Err(AuthnError::Invalid(format!("alg {:?} is not RS256", header.alg)));
    }
    let kid = header.kid.ok_or(AuthnError::NoKid)?;
    let key = cache.key_for(&kid).await?;

    let mut v = Validation::new(Algorithm::RS256);
    v.set_issuer(&[&cfg.issuer]);
    match &cfg.audience {
        Some(aud) => v.set_audience(&[aud]),
        None => v.validate_aud = false,
    }
    let data = decode::<Value>(token, &key, &v).map_err(|e| AuthnError::Invalid(e.to_string()))?;
    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::json;

    // A throwaway RSA keypair (2048-bit) for tests only — never a real key.
    const PRIV: &str = include_str!("../tests/test_rsa.pem");
    const PUB: &str = include_str!("../tests/test_rsa_pub.pem");
    const KID: &str = "test-kid-1";

    fn cache() -> JwksCache {
        let mut m = HashMap::new();
        m.insert(KID.to_string(), DecodingKey::from_rsa_pem(PUB.as_bytes()).unwrap());
        JwksCache::from_keys(m)
    }

    fn sign(claims: serde_json::Value) -> String {
        let mut h = Header::new(Algorithm::RS256);
        h.kid = Some(KID.to_string());
        encode(&h, &claims, &EncodingKey::from_rsa_pem(PRIV.as_bytes()).unwrap()).unwrap()
    }

    fn exp() -> i64 {
        4102444800 // 2100-01-01, safely in the future
    }

    #[tokio::test]
    async fn valid_token_verifies_and_returns_claims() {
        let cfg = AuthnConfig::from_issuer("https://idp.example/realms/a3t", Some("a3t-ee".into()));
        let tok = sign(json!({
            "iss": "https://idp.example/realms/a3t", "aud": "a3t-ee",
            "sub": "u-1", "preferred_username": "alice",
            "realm_access": {"roles": ["operator"]}, "exp": exp(),
        }));
        let claims = verify(&tok, &cache(), &cfg).await.unwrap();
        assert_eq!(claims["sub"], "u-1");
        assert_eq!(claims["preferred_username"], "alice");
        assert_eq!(claims["realm_access"]["roles"][0], "operator");
    }

    #[tokio::test]
    async fn wrong_issuer_is_rejected() {
        let cfg = AuthnConfig::from_issuer("https://idp.example/realms/a3t", None);
        let tok = sign(json!({"iss": "https://evil.example", "sub": "u", "exp": exp()}));
        assert!(matches!(verify(&tok, &cache(), &cfg).await, Err(AuthnError::Invalid(_))));
    }

    #[tokio::test]
    async fn expired_token_is_rejected() {
        let cfg = AuthnConfig::from_issuer("https://idp.example/realms/a3t", None);
        let tok = sign(json!({"iss": "https://idp.example/realms/a3t", "sub": "u", "exp": 1000}));
        assert!(matches!(verify(&tok, &cache(), &cfg).await, Err(AuthnError::Invalid(_))));
    }

    #[tokio::test]
    async fn wrong_audience_is_rejected() {
        let cfg = AuthnConfig::from_issuer("https://idp.example/realms/a3t", Some("a3t-ee".into()));
        let tok = sign(json!({"iss": "https://idp.example/realms/a3t", "aud": "someone-else", "sub": "u", "exp": exp()}));
        assert!(matches!(verify(&tok, &cache(), &cfg).await, Err(AuthnError::Invalid(_))));
    }

    #[tokio::test]
    async fn unknown_kid_without_jwks_fails_closed() {
        // A token with a kid the cache doesn't know and no reachable jwks_uri.
        let cfg = AuthnConfig::from_issuer("https://idp.example/realms/a3t", None);
        let mut h = Header::new(Algorithm::RS256);
        h.kid = Some("other-kid".into());
        let tok = encode(&h, &json!({"iss":"https://idp.example/realms/a3t","sub":"u","exp":exp()}),
            &EncodingKey::from_rsa_pem(PRIV.as_bytes()).unwrap()).unwrap();
        assert!(verify(&tok, &cache(), &cfg).await.is_err());
    }
}
