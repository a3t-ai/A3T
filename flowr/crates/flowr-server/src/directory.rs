//! `GET /directory/users` — the people a member can be chosen from.
//!
//! Adding someone to a project or an org needs their user id, but the full IdP
//! user list (`/security/idp/users`) is InfoSec's. Whoever may add members — the
//! owner of a project, an admin of an org, InfoSec — gets this narrower list
//! instead: id, name and email of the enabled users in the sign-in realm, and
//! nothing else (no roles, no attributes, no disabled accounts). Anyone else: 403.

use std::sync::Arc;

use axum::{extract::Extension, http::StatusCode, response::{IntoResponse, Response}, Json};
use serde_json::{json, Value};

use crate::auth::AuthConfig;

fn refuse(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// Whether `user` may add members anywhere: owner of a project (personal, or as
/// a member with role owner), or admin of an org.
async fn adds_members(config: &AuthConfig, user: &str) -> bool {
    let db = config.platform.get().await;
    let rows = |b: Vec<arrow_array::RecordBatch>| flowr_ce::handlers::call_lancedb::batches_to_json(&b);
    let q = user.replace(['"', '\\', '\''], "");
    if let Ok(b) = db.query("org_members", Some(&format!("user_id = \"{q}\"")), 200).await {
        if rows(b).iter().any(|m| m.get("role").and_then(|r| r.as_str()) == Some("admin")) {
            return true;
        }
    }
    if let Ok(b) = db.query("projects", None, 100_000).await {
        let owns = |p: &Value| {
            let s = |k: &str| p.get(k).and_then(|v| v.as_str()).unwrap_or_default();
            (s("owner_kind") == "user" && s("owner_id") == q)
                || serde_json::from_str::<Vec<Value>>(s("members")).unwrap_or_default().iter()
                    .any(|m| m.get("id").and_then(|i| i.as_str()) == Some(q.as_str()) && m.get("role").and_then(|r| r.as_str()) == Some("owner"))
        };
        if rows(b).iter().any(owns) {
            return true;
        }
    }
    false
}

pub async fn directory_users_handler(
    Extension(config): Extension<Arc<AuthConfig>>,
    Extension(idp): Extension<crate::IdpAdminState>,
    principal: Option<Extension<flowr_policy::Principal>>,
) -> Response {
    use flowr_policy::{roles, Principal};
    let Some(Extension(Principal::Human { user_id, roles: r, .. })) = principal else {
        return refuse(StatusCode::UNAUTHORIZED, "Sign in first.");
    };
    let infosec = r.iter().any(|x| x == roles::SUPER_ADMIN || x == roles::SECURITY_ADMIN);
    if !infosec && !adds_members(&config, &user_id).await {
        return refuse(StatusCode::FORBIDDEN, "Only someone who can add members (a project owner or an org admin) can list people.");
    }
    #[cfg(feature = "ee")]
    {
        use a3t_security::oidc_admin as kc;
        let Some(cfg) = kc::IdpAdminConfig::from_issuer(&idp.issuer, idp.admin_user.clone(), idp.admin_password.clone()) else {
            return Json(json!({ "count": 0, "users": [] })).into_response();
        };
        let token = match kc::admin_token(&idp.client, &cfg).await {
            Ok(t) => t,
            Err(e) => return refuse(StatusCode::BAD_GATEWAY, &e.to_string()),
        };
        match kc::list_users(&idp.client, &cfg, &token).await {
            Ok(users) => {
                let s = |u: &Value, k: &str| u.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let people: Vec<Value> = users.iter()
                    .filter(|u| u.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false))
                    .map(|u| {
                        let name = [s(u, "firstName"), s(u, "lastName")].join(" ").trim().to_string();
                        json!({ "id": s(u, "id"), "email": s(u, "email"), "name": if name.is_empty() { s(u, "username") } else { name } })
                    }).collect();
                Json(json!({ "count": people.len(), "users": people })).into_response()
            }
            Err(e) => refuse(StatusCode::BAD_GATEWAY, &e.to_string()),
        }
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = (idp, user_id);
        Json(json!({ "count": 0, "users": [] })).into_response()
    }
}
