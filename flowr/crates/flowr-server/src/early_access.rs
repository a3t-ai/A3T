//! Early-access requests from the sign-in page (`POST /auth/early-access`).
//!
//! The visitor has no account, so this is public like `/auth/forgot`. It can't go
//! through a webhook: every webhook needs a token or the trigger's secret, and a
//! page in a browser can hold neither. What anyone may send is checked and capped
//! here, the time and status are the server's, and the request is saved to the
//! platform table `early_access_requests` (where `a3t/flow_a3t_early_access`
//! lists them to an admin). Limits: one request per address a minute, and
//! `PER_CLIENT` per client address in `WINDOW`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{extract::Extension, http::{HeaderMap, StatusCode}, response::{IntoResponse, Response}, Json};
use serde::Deserialize;
use serde_json::json;
use tracing::{info, warn};

use crate::auth::AuthConfig;

const TABLE: &str = "early_access_requests";
const PER_ADDRESS: Duration = Duration::from_secs(60);
const WINDOW: Duration = Duration::from_secs(600);
const PER_CLIENT: usize = 5;

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct EarlyAccessBody {
    pub name: String,
    pub email: String,
    pub company: String,
    pub use_case: String,
    /// The sign-in provider the page says vouched for the email (as the page
    /// reports it; nothing here checks it).
    pub provider: String,
    pub email_verified: bool,
}

fn cap(s: &str, n: usize) -> String {
    s.trim().chars().filter(|c| !c.is_control() || *c == '\n').take(n).collect()
}

/// local@domain.tld with no spaces, the same shape the flow accepted.
fn email_ok(e: &str) -> bool {
    let Some((local, domain)) = e.split_once('@') else { return false };
    let Some((host, tld)) = domain.rsplit_once('.') else { return false };
    !e.chars().any(char::is_whitespace)
        && !domain.contains('@')
        && (1..=120).contains(&local.len())
        && (1..=120).contains(&host.len())
        && (1..=40).contains(&tld.len())
}

/// The visitor's address as the ingress saw it.
fn client(headers: &HeaderMap) -> String {
    headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.split(',').next())
        .or_else(|| headers.get("x-real-ip").and_then(|v| v.to_str().ok()))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Whether this request goes over a limit (and, if not, counts it).
fn limited(email: &str, ip: &str, now: Instant) -> bool {
    static SEEN: OnceLock<Mutex<(HashMap<String, Instant>, HashMap<String, Vec<Instant>>)>> = OnceLock::new();
    let mut g = SEEN.get_or_init(Default::default).lock().unwrap();
    let (by_email, by_ip) = &mut *g;
    by_email.retain(|_, t| now.duration_since(*t) < PER_ADDRESS);
    by_ip.retain(|_, ts| { ts.retain(|t| now.duration_since(*t) < WINDOW); !ts.is_empty() });
    if by_email.contains_key(email) || by_ip.get(ip).is_some_and(|ts| ts.len() >= PER_CLIENT) {
        return true;
    }
    by_email.insert(email.to_string(), now);
    by_ip.entry(ip.to_string()).or_default().push(now);
    false
}

fn refuse(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "ok": false, "error": msg }))).into_response()
}

pub async fn early_access_handler(
    Extension(config): Extension<Arc<AuthConfig>>,
    headers: HeaderMap,
    Json(body): Json<EarlyAccessBody>,
) -> Response {
    let email = body.email.trim().to_ascii_lowercase();
    if !email_ok(&email) {
        return refuse(StatusCode::BAD_REQUEST, "A valid email address is required.");
    }
    if limited(&email, &client(&headers), Instant::now()) {
        return refuse(StatusCode::TOO_MANY_REQUESTS, "We already have a request from you. Try again in a few minutes.");
    }
    let id = format!("ea-{}", uuid::Uuid::now_v7().simple());
    let record = json!({
        "id": id,
        "name": cap(&body.name, 200),
        "email": email,
        "company": cap(&body.company, 200),
        "use_case": cap(&body.use_case, 2000),
        "provider": cap(&body.provider, 40),
        "email_verified": body.email_verified.to_string(),
        "status": "new",
        "created_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
    });
    let platform = config.platform.get().await;
    match platform.insert(TABLE, &[record]).await {
        Ok(_) => {
            info!(request = %id, "early access: request saved");
            Json(json!({ "ok": true, "id": id })).into_response()
        }
        Err(e) => {
            warn!(error = %e, "early access: could not save the request");
            refuse(StatusCode::SERVICE_UNAVAILABLE, "We couldn't save your request just now. Please try again in a few minutes.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_the_email_shape() {
        assert!(email_ok("ann@example.com"));
        assert!(!email_ok("not-an-email"));
        assert!(!email_ok("a b@example.com"));
        assert!(!email_ok("a@b@example.com"));
        assert!(!email_ok("ann@example"));
    }

    #[test]
    fn caps_fields() {
        assert_eq!(cap("  hi\u{0}there  ", 200), "hithere");
        assert_eq!(cap(&"x".repeat(300), 200).len(), 200);
    }

    #[test]
    fn limits_per_address_and_client() {
        let t = Instant::now();
        assert!(!limited("one@t.io", "198.51.100.7", t));
        assert!(limited("one@t.io", "198.51.100.8", t), "same address within a minute");
        for i in 0..4 { assert!(!limited(&format!("k{i}@t.io"), "198.51.100.9", t)); }
        assert!(!limited("k4@t.io", "198.51.100.9", t));
        assert!(limited("k5@t.io", "198.51.100.9", t), "sixth from one client in ten minutes");
    }
}
