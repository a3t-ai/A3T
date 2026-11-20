//! Sign-in sessions (ID-6): one row per sign-in in the platform table
//! `auth_sessions`, shared by every pod.
//!
//! - Every access token and refresh token names its session (`sid`).
//! - A refresh token is good once. `/auth/refresh` swaps it for a new one (`jti`)
//!   and remembers the one it replaced. Presenting a replaced token again means it
//!   was copied: the whole session is revoked. The one exception is a second tab
//!   or a retried request presenting the token replaced in the last
//!   [`GRACE_SECS`]: that gets `409 refresh_stale` (read the newer token from
//!   storage) and revokes nothing.
//! - A session ends at sign-in time + the refresh lifetime, however often it is
//!   refreshed. People sign in again at least that often.
//! - Sign-out (`POST /auth/logout`) revokes the session. Each pod reloads the set
//!   of revoked sessions every [`RELOAD_SECS`], and access tokens of a revoked
//!   session are refused from then on (at once on the pod that revoked it).

use std::collections::HashSet;
use std::sync::RwLock;

use serde_json::{json, Map, Value};
use tracing::{info, warn};

use crate::auth::AuthConfig;

pub const TABLE: &str = "auth_sessions";
pub const GRACE_SECS: i64 = 20;
pub const RELOAD_SECS: u64 = 10;
const COLUMNS: [&str; 10] = ["sid", "sub", "kind", "jti", "prev_jti", "rotated_at", "revoked", "reason", "created_at", "expires_at"];

/// Make the table if it isn't there (on ADB a read of a missing table fails).
pub async fn ensure_table(config: &AuthConfig) -> Result<(), String> {
    let schema: Map<String, Value> = COLUMNS.iter().map(|c| (c.to_string(), json!("string"))).collect();
    config.platform.get().await.create_table(TABLE, Some(&schema), "create_if_not_exists").await.map(|_| ())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Epoch seconds as fixed-width text, so the text columns compare in time order.
fn stamp(t: i64) -> String {
    format!("{t:011}")
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn quote(s: &str) -> String {
    s.replace('\'', "''")
}

/// The sessions this pod knows to be revoked.
#[derive(Default)]
pub struct Revoked(RwLock<HashSet<String>>);

impl Revoked {
    pub fn contains(&self, sid: &str) -> bool {
        self.0.read().map(|s| s.contains(sid)).unwrap_or(false)
    }

    fn insert(&self, sid: &str) {
        if let Ok(mut s) = self.0.write() {
            s.insert(sid.to_string());
        }
    }

    /// Replace the set with the revoked rows that have not yet expired. A failed
    /// read keeps the set as it was.
    pub async fn reload(&self, config: &AuthConfig) {
        let db = config.platform.get().await;
        let filter = format!("revoked = '1' AND expires_at > '{}'", stamp(now()));
        match db.query(TABLE, Some(&filter), 1_000_000).await {
            Ok(batches) => {
                let rows = flowr_ce::handlers::call_lancedb::batches_to_json(&batches);
                let set: HashSet<String> = rows.iter().filter_map(|r| r.get("sid").and_then(|v| v.as_str()).map(str::to_string)).collect();
                if let Ok(mut s) = self.0.write() {
                    *s = set;
                }
            }
            Err(e) => warn!(error = %e, "sessions: revoked set not reloaded"),
        }
    }

    /// Delete sessions that ended more than a day ago.
    pub async fn sweep(config: &AuthConfig) {
        let db = config.platform.get().await;
        if let Err(e) = db.delete(TABLE, &format!("expires_at < '{}'", stamp(now() - 86_400))).await {
            warn!(error = %e, "sessions: expired rows not deleted");
        }
    }
}

/// A new session for `sub`, ending `ttl` seconds from now. Returns (sid, jti).
pub async fn open(config: &AuthConfig, sub: &str, kind: &str, ttl: u64) -> Result<(String, String, i64), String> {
    let (sid, jti, t) = (new_id(), new_id(), now());
    let expires = t + ttl as i64;
    let row = json!({
        "sid": sid, "sub": sub, "kind": kind, "jti": jti, "prev_jti": "",
        "rotated_at": stamp(t), "revoked": "0", "reason": "",
        "created_at": stamp(t), "expires_at": stamp(expires),
    });
    config.platform.get().await.insert(TABLE, &[row]).await?;
    Ok((sid, jti, expires))
}

pub enum Turn {
    /// The new jti, and when the session ends.
    Rotated(String, i64),
    /// The token was replaced moments ago (another tab, a retried request).
    Stale,
    Refused(&'static str),
}

async fn row(config: &AuthConfig, sid: &str) -> Result<Option<Value>, String> {
    let b = config.platform.get().await.query(TABLE, Some(&format!("sid = '{}'", quote(sid))), 1).await?;
    Ok(flowr_ce::handlers::call_lancedb::batches_to_json(&b).into_iter().next())
}

/// Swap refresh token `jti` of session `sid` (held by `sub`) for a new one.
pub async fn rotate(config: &AuthConfig, sid: &str, sub: &str, jti: &str) -> Result<Turn, String> {
    let Some(r) = row(config, sid).await? else { return Ok(Turn::Refused("no such session")) };
    let s = |k: &str| r.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let t = now();
    let expires: i64 = s("expires_at").parse().unwrap_or(0);
    if s("sub") != sub || s("kind") != "signin" {
        return Ok(Turn::Refused("not this session"));
    }
    if s("revoked") == "1" {
        return Ok(Turn::Refused("revoked"));
    }
    if expires <= t {
        return Ok(Turn::Refused("ended"));
    }
    if s("jti") == jti {
        let next = new_id();
        let mut v = Map::new();
        v.insert("jti".into(), Value::String(next.clone()));
        v.insert("prev_jti".into(), Value::String(jti.to_string()));
        v.insert("rotated_at".into(), Value::String(stamp(t)));
        config.platform.get().await.update(TABLE, &format!("sid = '{}'", quote(sid)), &v).await?;
        return Ok(Turn::Rotated(next, expires));
    }
    let rotated: i64 = s("rotated_at").parse().unwrap_or(0);
    if s("prev_jti") == jti && t - rotated <= GRACE_SECS {
        return Ok(Turn::Stale);
    }
    // A refresh token used twice: someone else holds a copy.
    warn!(sid = %sid, user = %sub, "sessions: refresh token reused; session revoked");
    revoke(config, sid, "refresh token reused").await?;
    Ok(Turn::Refused("reused"))
}

/// End session `sid` everywhere.
pub async fn revoke(config: &AuthConfig, sid: &str, reason: &str) -> Result<(), String> {
    config.revoked.insert(sid);
    let mut v = Map::new();
    v.insert("revoked".into(), Value::String("1".into()));
    v.insert("reason".into(), Value::String(reason.to_string()));
    config.platform.get().await.update(TABLE, &format!("sid = '{}'", quote(sid)), &v).await?;
    info!(sid = %sid, reason = %reason, "session revoked");
    Ok(())
}

/// Reload the revoked set every [`RELOAD_SECS`] and sweep old rows hourly.
pub fn spawn_upkeep(config: std::sync::Arc<AuthConfig>) {
    tokio::spawn(async move {
        let mut tick: u64 = 0;
        while let Err(e) = ensure_table(&config).await {
            warn!(error = %e, "sessions: table not ready; retrying");
            tokio::time::sleep(std::time::Duration::from_secs(RELOAD_SECS)).await;
        }
        loop {
            config.revoked.reload(&config).await;
            if tick % (3600 / RELOAD_SECS) == 0 {
                Revoked::sweep(&config).await;
            }
            tick += 1;
            tokio::time::sleep(std::time::Duration::from_secs(RELOAD_SECS)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{create_refresh_token, create_token, session_of, validate_refresh, validate_token};
    use std::sync::Arc;

    fn config() -> AuthConfig {
        let dir = std::env::temp_dir().join(format!("a3t-sessions-{}", new_id()));
        AuthConfig {
            jwt_secret: "0123456789abcdef0123456789abcdef-test".into(),
            token_expiry_secs: 3600,
            refresh_expiry_secs: 7 * 86_400,
            a3t_db_path: dir.to_string_lossy().to_string(),
            platform: flowr_ce::platform_db::PlatformSource::Fixed(Arc::new(flowr_ce::platform_db::LanceDbPlatform::new(dir.to_string_lossy().to_string()))),
            resource_uri: "https://console.test".into(),
            oidc_issuer: String::new(),
            oidc_client_id: String::new(),
            revoked: Arc::new(Revoked::default()),
        }
    }

    #[test]
    fn access_tokens_need_this_issuer_audience_and_a_session() {
        let c = config();
        let t = create_token(&c, "s1", "u1", "org", "viewer", "e", "n").unwrap();
        assert_eq!(validate_token(&t, &c).unwrap().sid, "s1");
        // Another issuer (same secret, same audience) is refused.
        let mut other = config();
        other.jwt_secret = c.jwt_secret.clone();
        other.resource_uri = "https://elsewhere.test".into();
        let foreign = create_token(&other, "s1", "u1", "org", "viewer", "e", "n").unwrap();
        assert!(validate_token(&foreign, &c).is_err());
        // No session named: refused.
        let none = create_token(&c, "", "u1", "org", "viewer", "e", "n").unwrap();
        assert!(validate_token(&none, &c).is_err());
        // A revoked session: refused.
        c.revoked.insert("s1");
        assert!(validate_token(&t, &c).is_err());
        assert_eq!(session_of(&t, &c).as_deref(), Some("s1"));
    }

    #[test]
    fn a_refresh_token_is_not_an_access_token_nor_the_reverse() {
        let c = config();
        let r = create_refresh_token(&c, "u1", "s1", "j1", now() + 60).unwrap();
        assert!(validate_token(&r, &c).is_err());
        assert_eq!(validate_refresh(&r, &c, false).unwrap().jti, "j1");
        let a = create_token(&c, "s1", "u1", "org", "viewer", "e", "n").unwrap();
        assert!(validate_refresh(&a, &c, false).is_err());
        // Expired: refused for refresh, still names its session for sign-out.
        let old = create_refresh_token(&c, "u1", "s1", "j1", now() - 120).unwrap();
        assert!(validate_refresh(&old, &c, false).is_err());
        assert!(validate_refresh(&old, &c, true).is_ok());
    }

    #[tokio::test]
    async fn a_refresh_token_works_once_and_reuse_ends_the_session() {
        let c = config();
        let (sid, j1, _) = open(&c, "u1", "signin", 3600).await.unwrap();
        let Turn::Rotated(j2, _) = rotate(&c, &sid, "u1", &j1).await.unwrap() else { panic!("first use rotates") };
        // The replaced token again, at once: stale, nothing revoked.
        assert!(matches!(rotate(&c, &sid, "u1", &j1).await.unwrap(), Turn::Stale));
        // Someone else's sub, or a made-up jti: refused, and the made-up one revokes.
        assert!(matches!(rotate(&c, &sid, "u2", &j2).await.unwrap(), Turn::Refused(_)));
        assert!(matches!(rotate(&c, &sid, "u1", "forged").await.unwrap(), Turn::Refused("reused")));
        assert!(c.revoked.contains(&sid));
        // The current token no longer works either.
        assert!(matches!(rotate(&c, &sid, "u1", &j2).await.unwrap(), Turn::Refused("revoked")));
        // Another pod learns of it on reload.
        let other = AuthConfig { revoked: Arc::new(Revoked::default()), ..c.clone() };
        other.revoked.reload(&other).await;
        assert!(other.revoked.contains(&sid));
    }

    #[tokio::test]
    async fn sign_out_revokes_and_an_act_session_cannot_refresh() {
        let c = config();
        let (sid, jti, _) = open(&c, "u1", "act", 600).await.unwrap();
        assert!(matches!(rotate(&c, &sid, "u1", &jti).await.unwrap(), Turn::Refused(_)));
        let (sid2, jti2, _) = open(&c, "u1", "signin", 600).await.unwrap();
        revoke(&c, &sid2, "signed out").await.unwrap();
        assert!(matches!(rotate(&c, &sid2, "u1", &jti2).await.unwrap(), Turn::Refused("revoked")));
    }

    #[test]
    fn stamps_sort_in_time_order() {
        assert!(stamp(999) < stamp(1_000));
        assert!(stamp(1_795_000_000) < stamp(10_000_000_000));
    }

    #[test]
    fn revoked_set_answers_for_what_it_holds() {
        let r = Revoked::default();
        assert!(!r.contains("a"));
        r.insert("a");
        assert!(r.contains("a"));
        assert!(!r.contains(""));
    }
}
