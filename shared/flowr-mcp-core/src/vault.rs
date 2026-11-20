//! Secrets (RT-7): encrypted at rest, and given only to their own.
//!
//! - **At rest.** A `secret` resource's values are stored as
//!   `enc:v1:<base64 nonce‖ciphertext>` (AES-256-GCM) under `FLOWR_SECRETS_KEY`
//!   (32 random bytes, base64, from a secret store; every FlowR that resolves
//!   secrets has it). A value saved before the key was set reads as it is and
//!   is sealed on its next save. Without a key, values are kept as given and
//!   FlowR says so at start.
//! - **Scope.** A secret is resolved only for a resource (or flow) of its own
//!   project; the platform's secrets (`a3t/…`, `system/…`, `default/…`) only for
//!   the platform's resources. A project can't name another project's or org's
//!   secret and get its value, whichever runtime runs it.

use std::sync::OnceLock;

use base64::Engine;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::Value;

const PREFIX: &str = "enc:v1:";
/// Namespaces whose secrets are the platform's.
pub const PLATFORM: &[&str] = &["a3t", "system", "default"];

fn key() -> Option<&'static LessSafeKey> {
    static K: OnceLock<Option<LessSafeKey>> = OnceLock::new();
    K.get_or_init(|| {
        let raw = std::env::var("FLOWR_SECRETS_KEY").ok().filter(|k| !k.trim().is_empty())?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(raw.trim()).ok().filter(|b| b.len() == 32)?;
        UnboundKey::new(&AES_256_GCM, &bytes).ok().map(LessSafeKey::new)
    }).as_ref()
}

/// Whether values are sealed at rest here.
pub fn sealing() -> bool {
    key().is_some()
}

/// Seal one value (already sealed: as it is; no key: as it is).
pub fn seal(plain: &str) -> String {
    if plain.starts_with(PREFIX) {
        return plain.to_string();
    }
    let Some(k) = key() else { return plain.to_string() };
    let mut nonce = [0u8; 12];
    if SystemRandom::new().fill(&mut nonce).is_err() {
        return plain.to_string();
    }
    let mut buf = plain.as_bytes().to_vec();
    if k.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut buf).is_err() {
        return plain.to_string();
    }
    let mut out = nonce.to_vec();
    out.extend(buf);
    format!("{PREFIX}{}", base64::engine::general_purpose::STANDARD.encode(out))
}

/// Open one value (not sealed: as it is).
pub fn open(stored: &str) -> Result<String, String> {
    let Some(b64) = stored.strip_prefix(PREFIX) else { return Ok(stored.to_string()) };
    let k = key().ok_or("this secret is sealed and this FlowR has no FLOWR_SECRETS_KEY")?;
    let raw = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|_| "a sealed value that doesn't decode")?;
    if raw.len() < 12 + 16 {
        return Err("a sealed value that is too short".into());
    }
    let (nonce, ct) = raw.split_at(12);
    let mut buf = ct.to_vec();
    let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| "bad nonce")?;
    let plain = k.open_in_place(nonce, Aad::empty(), &mut buf).map_err(|_| "a sealed value that doesn't open with this key")?;
    String::from_utf8(plain.to_vec()).map_err(|_| "not text".into())
}

/// Seal every string in a secret's config.
pub fn seal_config(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(seal(s)),
        Value::Array(a) => Value::Array(a.iter().map(seal_config).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), seal_config(x))).collect()),
        other => other.clone(),
    }
}

/// A resource's config as stored: a `secret`'s values sealed, others as given.
pub fn seal_for(type_ref: &str, config: Value) -> Value {
    if type_ref == "secret" { seal_config(&config) } else { config }
}

fn has_plain(v: &Value) -> bool {
    match v {
        Value::String(s) => !s.starts_with(PREFIX),
        Value::Array(a) => a.iter().any(has_plain),
        Value::Object(o) => o.values().any(has_plain),
        _ => false,
    }
}

/// At start: seal the secrets saved before the key was set. Returns how many.
pub async fn seal_existing(storage: &dyn flowr_storage::StorageBackend) -> usize {
    if !sealing() {
        tracing::warn!("secrets: FLOWR_SECRETS_KEY is not set, secret values are stored as given");
        return 0;
    }
    let mut n = 0;
    for ns in storage.resources().namespaces().await.unwrap_or_default() {
        for r in storage.resources().list(&ns).await.unwrap_or_default() {
            if r.type_ref == "secret" && has_plain(&r.config) {
                match storage.resources().update(&ns, &r.name, seal_config(&r.config)).await {
                    Ok(_) => n += 1,
                    Err(e) => tracing::warn!(secret = %format!("{ns}/{}", r.name), error = %e, "secret not sealed"),
                }
            }
        }
    }
    tracing::info!(sealed = n, "secrets: sealed at rest");
    n
}

/// `<namespace>/<name>`, or `default/<name>`.
pub fn split_ref(secret_ref: &str) -> (String, String) {
    match secret_ref.split_once('/') {
        Some((ns, n)) => (ns.to_string(), n.to_string()),
        None => ("default".to_string(), secret_ref.to_string()),
    }
}

/// May something in namespace `from` use the secret in namespace `secret_ns`?
pub fn in_scope(from: &str, secret_ns: &str) -> bool {
    from == secret_ns || (PLATFORM.contains(&secret_ns) && PLATFORM.contains(&from))
}

/// The value of `secret_ref` for something in namespace `from` (`value`,
/// `token` or `api_key` of the secret's config), checked and opened.
pub async fn resolve(storage: &dyn flowr_storage::StorageBackend, secret_ref: &str, from: &str) -> Result<String, String> {
    let (ns, name) = split_ref(secret_ref);
    if !in_scope(from, &ns) {
        tracing::warn!(secret = %secret_ref, from, "secret refused: out of scope");
        return Err(format!("the secret {secret_ref} isn't {from}'s: a secret is used only by its own project (the platform's by the platform)"));
    }
    let secret = storage.resources().get(&ns, &name).await.map_err(|_| format!("no secret {secret_ref}"))?;
    let raw = secret.config.get("value").or_else(|| secret.config.get("token")).or_else(|| secret.config.get("api_key"))
        .and_then(|v| v.as_str()).ok_or_else(|| format!("the secret {secret_ref} has no value"))?;
    open(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope() {
        assert!(in_scope("acme-web", "acme-web"));
        assert!(!in_scope("acme-web", "beta-crm"));
        assert!(!in_scope("acme-web", "a3t"), "a project doesn't take the platform's secrets");
        assert!(in_scope("a3t", "default"));
    }

    #[test]
    fn seal_and_open() {
        std::env::set_var("FLOWR_SECRETS_KEY", base64::engine::general_purpose::STANDARD.encode([7u8; 32]));
        let s = seal("sk-test-123");
        assert!(s.starts_with(PREFIX));
        assert_ne!(seal("sk-test-123"), s, "a fresh nonce each time");
        assert_eq!(open(&s).unwrap(), "sk-test-123");
        assert_eq!(open("plain").unwrap(), "plain");
        assert!(open(&format!("{PREFIX}AAAA")).is_err());
    }
}
