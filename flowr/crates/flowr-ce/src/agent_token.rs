//! An agent's identity on the wire (ID-4): when an agent calls a tool server
//! outside A3T, the call carries a short-lived token that names the agent and
//! the person it acts for, so the server can decide on both.
//!
//! - Header `A3T-Agent-Identity: <JWT>` (EdDSA): `sub` = `agent:<agent id>`,
//!   `act.sub` = the person (RFC 8693 actor claim; absent for a run nobody
//!   started), `aud` = the server's origin, `a3t.project`, `iss` = this A3T,
//!   five minutes long.
//! - Each FlowR signs with a key of its own, made at start and never written
//!   down; its public half goes to `agent_token_keys`, and every FlowR serves
//!   them all at `GET /.well-known/agent-jwks.json`, so a server verifies a
//!   token whichever FlowR made it.

use std::sync::{Arc, OnceLock};

use base64::Engine;
use ring::signature::KeyPair;
use serde_json::{json, Map, Value};

use flowr_storage::StorageBackend;

use crate::handlers::call_lancedb::{query_platform_table, DEFAULT_PLATFORM_DB_PATH, PLATFORM_DB_RESOURCE};
use crate::platform_db::open_platform_db;

pub const KEYS: &str = "agent_token_keys";
pub const HEADER: &str = "A3T-Agent-Identity";
const LIFE_SECS: i64 = 300;

struct Key {
    kid: String,
    pkcs8: Vec<u8>,
    public: Vec<u8>,
}

fn key() -> &'static Key {
    static K: OnceLock<Key> = OnceLock::new();
    K.get_or_init(|| {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("an Ed25519 key").as_ref().to_vec();
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(&pkcs8).expect("the key just made");
        let public = pair.public_key().as_ref().to_vec();
        let kid = format!("{}-{}", std::env::var("HOSTNAME").unwrap_or_else(|_| "flowr".into()), &hex::encode(&public)[..12]);
        Key { kid, pkcs8, public }
    })
}

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// The JWK of this process's key.
pub fn jwk() -> Value {
    let k = key();
    json!({ "kty": "OKP", "crv": "Ed25519", "x": b64(&k.public), "kid": k.kid, "alg": "EdDSA", "use": "sig" })
}

/// At start: publish this process's public key.
pub fn install(storage: Arc<dyn StorageBackend>) {
    tokio::spawn(async move {
        let db = open_platform_db(storage.as_ref(), Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
        let schema: Map<String, Value> = ["kid", "jwk", "created_at"].iter().map(|c| (c.to_string(), json!("string"))).collect();
        for attempt in 0..10 {
            let ok = async {
                db.create_table(KEYS, Some(&schema), "create_if_not_exists").await?;
                db.upsert(KEYS, &["kid"], &[json!({ "kid": key().kid, "jwk": jwk().to_string(), "created_at": chrono::Utc::now().to_rfc3339() })]).await
            }.await;
            match ok {
                Ok(_) => { tracing::info!(kid = %key().kid, "agent identity: signing key published"); return; }
                Err(e) if attempt == 9 => tracing::warn!(error = %e, "agent identity: signing key not published"),
                Err(_) => tokio::time::sleep(std::time::Duration::from_secs(5)).await,
            }
        }
    });
}

/// Every FlowR's public keys, as a JWK set (the last 30 days).
pub async fn jwks(s: &dyn StorageBackend) -> Value {
    let since = (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339();
    let mut keys: Vec<Value> = query_platform_table(s, KEYS, None, 10_000).await.unwrap_or_default().into_iter()
        .filter(|r| r.get("created_at").and_then(|v| v.as_str()).is_some_and(|t| t >= since.as_str()))
        .filter_map(|r| r.get("jwk").and_then(|v| v.as_str()).and_then(|t| serde_json::from_str(t).ok()))
        .collect();
    if !keys.iter().any(|k| k["kid"] == key().kid) {
        keys.push(jwk());
    }
    json!({ "keys": keys })
}

/// The origin a token is for (`scheme://host[:port]`).
pub fn audience(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => format!("{scheme}://{}", rest.split('/').next().unwrap_or_default()),
        None => url.to_string(),
    }
}

/// A token for `agent` acting for `person`, for the server at `url`.
pub fn mint(agent: &str, person: Option<&str>, project: Option<&str>, url: &str) -> Result<String, String> {
    let k = key();
    let now = chrono::Utc::now().timestamp();
    let mut claims = json!({
        "iss": std::env::var("MCP_RESOURCE_URI").unwrap_or_else(|_| "a3t".into()),
        "sub": format!("agent:{agent}"), "aud": audience(url),
        "iat": now, "exp": now + LIFE_SECS, "jti": uuid::Uuid::now_v7().to_string(),
        "a3t": { "agent": agent, "project": project },
    });
    if let Some(p) = person.filter(|p| !p.is_empty()) {
        claims["act"] = json!({ "sub": p });
    }
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA);
    header.kid = Some(k.kid.clone());
    jsonwebtoken::encode(&header, &claims, &jsonwebtoken::EncodingKey::from_ed_der(&k.pkcs8)).map_err(|e| e.to_string())
}

/// The agent and person behind a principal, for [`mint`].
pub fn of_principal(p: &flowr_policy::Principal) -> Option<(String, Option<String>)> {
    match p {
        flowr_policy::Principal::Agent { agent_id, on_behalf_of, .. } => {
            // `agent:<agent id>:<conversation>` → the agent id.
            let id = agent_id.0.trim_start_matches("agent:").split(':').next().unwrap_or_default().to_string();
            Some((id, on_behalf_of.clone()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_names_the_agent_and_its_person_and_verifies_with_the_jwk() {
        let t = mint("agt-1", Some("u-1"), Some("acme-web"), "https://tools.example.com/mcp/message").unwrap();
        let jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk()).unwrap();
        let mut v = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::EdDSA);
        v.set_audience(&["https://tools.example.com"]);
        let d = jsonwebtoken::decode::<Value>(&t, &jsonwebtoken::DecodingKey::from_jwk(&jwk).unwrap(), &v).expect("it verifies");
        assert_eq!(d.claims["sub"], "agent:agt-1");
        assert_eq!(d.claims["act"]["sub"], "u-1");
        assert_eq!(d.claims["a3t"]["project"], "acme-web");
    }

    #[test]
    fn the_agent_id_comes_from_its_session() {
        let p = flowr_policy::Principal::Agent { session: None, memory_branch: None, agent_id: flowr_policy::AgentId("agent:agt-9:s-1".into()),
            on_behalf_of: Some("u".into()), flow_id: None, roles: vec![], owner: None };
        assert_eq!(of_principal(&p), Some(("agt-9".into(), Some("u".into()))));
    }
}
