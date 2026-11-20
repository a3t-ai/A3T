//! FlowR EE Server
//!
//! Extends ClawdFlow CE with:
//! - JWT authentication
//! - Casbin RBAC (pluggable: internal/external/flow)
//! - Config file (flowr.yaml)
//! - Settings table in system DB
//! - Auth routes: /auth/login, /auth/signup, /auth/me
//! - CORS

mod auth;
mod auth_routes;
mod sso;
mod impersonate;
mod forgot;
mod first_password;
mod change_password;
mod refresh;
mod sessions;
mod replay;
mod directory;
mod people;
mod config;

use flowr_ce::flowr_storage::StorageBackend;

use axum::{
    middleware,
    routing::{get, post},
    Extension, Router,
    response::IntoResponse,
};
use flowr_ce::{
    build_ce_router, init_storage, restore_triggers, run_stdio, serve, spawn_worker, ServerConfig,
};
use std::sync::Arc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use auth::{jwt_auth_middleware, AuthConfig};
use auth_routes::{login_handler, me_handler, signup_handler};
use config::FlowrConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().collect();
    let mode = get_arg(&args, "--mode").unwrap_or_else(|| "stdio".to_string());
    let port: u16 = get_arg(&args, "--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(3002);
    // MCP URLs on this port are FlowR itself (served in-process).
    flowr_mcp_core::set_self_port(port);
    let config_path = get_arg(&args, "--config").unwrap_or_else(|| "config/flowr.yaml".to_string());

    // Load config
    let mut flowr_config = FlowrConfig::load(&config_path);
    flowr_config.apply_env_overrides();

    // Authentication is unconditional. There is deliberately no switch to turn
    // it off: `bootstrap::seed_platform` provisions the first credential, so no
    // unauthenticated window is ever needed to install the platform.
    info!(mode = %mode, auth = true, "Starting FlowR server");

    let server_config = ServerConfig {
        port,
        mode: mode.clone(),
        data_dir: std::env::var("CLAWDFLOW_DATA_DIR").unwrap_or_else(|_| {
            dirs::data_local_dir()
                .map(|p| p.join("clawdflow").to_string_lossy().to_string())
                .unwrap_or_else(|| ".clawdflow".to_string())
        }),
    };

    // Storage backend selection. CE always uses embedded LanceDB. EE may request an
    // external backend (Dragonfly/Iggy/…) via config/env; those connectors are wired
    // in later phases, so anything other than lancedb logs and falls back for now.
    #[cfg(feature = "ee")]
    let requested_backend = flowr_config.storage.backend.to_lowercase();
    #[cfg(not(feature = "ee"))]
    let requested_backend = "lancedb".to_string();
    if !requested_backend.starts_with("lancedb") {
        warn!(backend = %requested_backend, "external storage backend requested; connector not yet wired in this build — using embedded LanceDB");
    }
    info!(active = "lancedb", requested = %requested_backend, "storage backend selected");
    let storage: Arc<dyn StorageBackend> = init_storage(&server_config.data_dir).await?;

    // Seed default settings
    let settings = storage.settings();
    let defaults = vec![
        ("a3t.db_path", serde_json::json!(flowr_config.a3t.db_path), "config"),
        ("auth.token_expiry", serde_json::json!(flowr_config.auth.token_expiry), "config"),
        ("security.policy_hook", serde_json::json!(flowr_config.security.policy_hook), "config"),
        ("security.policies_dir", serde_json::json!(flowr_config.security.policies_dir), "config"),
        ("security.audit_exporter", serde_json::json!(flowr_config.security.audit_exporter), "config"),
        ("security.authn", serde_json::json!(flowr_config.security.authn), "config"),
        ("security.oidc_issuer", serde_json::json!(flowr_config.security.oidc_issuer), "config"),
        ("security.oidc_audience", serde_json::json!(flowr_config.security.oidc_audience), "config"),
        ("security.oidc_admin_user", serde_json::json!(flowr_config.security.oidc_admin_user), "config"),
    ];
    let seeded = settings.seed_defaults(&defaults).await
        .unwrap_or_else(|e| { warn!(error = %e, "Failed to seed settings"); 0 });
    if seeded > 0 { info!(count = seeded, "Settings seeded"); }

    // E0.6: install the AESOP policy hook named by the effective `security.policy_hook`
    // SETTING (config seeds it; the settings CRUD is the source of truth, so a
    // change takes effect on restart). This one setting drives the CE→EE
    // enforcement upgrade. CE ships allow_all / deny_all; EE adds the "cedar" arm.
    let hook_setting = settings.get_string("security.policy_hook").await
        .ok().flatten()
        .unwrap_or_else(|| flowr_config.security.policy_hook.clone());
    let policies_dir = settings.get_string("security.policies_dir").await
        .ok().flatten()
        .unwrap_or_else(|| flowr_config.security.policies_dir.clone());
    // E2 #4: resolve the AuthN scheme + OIDC params up front (before `storage`
    // is moved into build_ce_router). Precedence: env → settings DB → config, so
    // a deployment can flip AuthN via env even after the setting was seeded (the
    // settings-seed footgun otherwise pins a DB value across restarts).
    let env_or = |var: &str, setting: Option<String>, cfg: &str| -> String {
        std::env::var(var).ok().filter(|s| !s.is_empty())
            .or(setting.filter(|s| !s.is_empty()))
            .unwrap_or_else(|| cfg.to_string())
    };
    let authn = env_or("FLOWR_SECURITY_AUTHN",
        settings.get_string("security.authn").await.ok().flatten(), &flowr_config.security.authn);
    let oidc_issuer = env_or("FLOWR_SECURITY_OIDC_ISSUER",
        settings.get_string("security.oidc_issuer").await.ok().flatten(), &flowr_config.security.oidc_issuer);
    let oidc_audience = env_or("FLOWR_SECURITY_OIDC_AUDIENCE",
        settings.get_string("security.oidc_audience").await.ok().flatten(), &flowr_config.security.oidc_audience);
    // Keycloak Admin API creds for IdP management (create users from the UI).
    // User is env→settings→config; the PASSWORD is env-only (a secret, never seeded).
    let oidc_admin_user = env_or("FLOWR_SECURITY_OIDC_ADMIN_USER",
        settings.get_string("security.oidc_admin_user").await.ok().flatten(), &flowr_config.security.oidc_admin_user);
    let oidc_admin_password = std::env::var("FLOWR_SECURITY_OIDC_ADMIN_PASSWORD").unwrap_or_default();
    let oidc_client_id = env_or("FLOWR_SECURITY_OIDC_CLIENT_ID",
        settings.get_string("security.oidc_client_id").await.ok().flatten(), &flowr_config.security.oidc_client_id);
    // E2 #2c: gather InfoSec-owned `type_ref: cedar` policy resources from the
    // registry to layer onto the baked baseline. `config.cedar` holds the source
    // (see §6.3). Empty → the hook degrades to exactly the baseline pack.
    let registry_policies = collect_registry_cedar_policies(&storage).await;
    if !registry_policies.is_empty() {
        info!(count = registry_policies.len(), "loaded registry cedar policy resources");
    }
    let (schema_base, schema_fragments) = collect_registry_schema(&storage).await;
    if schema_base.is_some() || !schema_fragments.is_empty() {
        info!(base_override = schema_base.is_some(), fragments = schema_fragments.len(), "loaded registry cedar schema (base + fragments)");
    }
    let effective_hook = install_policy_hook(&hook_setting, &policies_dir, &registry_policies, schema_base.as_deref(), &schema_fragments);
    let edition = if effective_hook == "cedar" { "ee" } else { "ce" };
    // `enforcing` = a non-permissive hook is active (cedar or deny_all).
    let enforcing = effective_hook != "allow_all";
    let _ = EDITION_INFO.set(serde_json::json!({
        "edition": edition,
        "enforcing": enforcing,
        "policy_hook": effective_hook,
        "configured_policy_hook": hook_setting,
        "features": if cfg!(feature = "ee") { serde_json::json!(["cedar"]) } else { serde_json::json!([]) },
    }));
    info!(policy_hook = %effective_hook, edition = %edition, "AESOP policy hook installed");
    // HS256 tokens (console login, Act as, refresh) are signed with this secret.
    // A server that accepts them (`hs256`, `dual`) and enforces policy would take
    // a token anyone can mint with a known secret, for any role: it refuses to
    // start. One that takes OIDC tokens only never accepts them, so it warns.
    if let Some(why) = flowr_config.auth.weak_jwt_secret() {
        let accepts_hs256 = authn != "oidc";
        if enforcing && accepts_hs256 {
            anyhow::bail!("{why}; set it from a secret store (32+ random bytes) before starting with policy enforced and HS256 tokens accepted (security.authn = {authn})");
        }
        warn!(authn = %authn, enforcing, "{why}; tolerated only because this server enforces no policy or accepts OIDC tokens only");
    }
    // ID-6: an OIDC token must be minted for A3T. Without an audience to check,
    // any token the realm issues to any client would do.
    if enforcing && authn != "hs256" && oidc_audience.trim().is_empty() {
        anyhow::bail!("security.oidc_audience (FLOWR_SECURITY_OIDC_AUDIENCE) is empty; set it to the audience A3T's tokens carry before starting with policy enforced and OIDC tokens accepted (security.authn = {authn})");
    }
    // Projects: with policy enforcing, a caller sees and changes only the
    // namespaces (projects), runs and runtimes it may — the rules of the A3T
    // project flows, enforced at every boundary.
    // Agent keys (a3tk_…) are looked up in the platform tables, enforcing or not.
    flowr_ce::agent_keys::install(storage.clone());
    // AUD-1: every decision this pod makes goes to the durable log in ADB.
    #[cfg(feature = "ee")]
    flowr_ce::decision_log::install(storage.clone());
    // RT-1: where project flows and agents may call out to (each org's egress files).
    flowr_ce::egress_rules::install(storage.clone());
    // GOV-5: sign-in role maps live in each org's access/org.yaml.
    tokio::spawn({
        let s = storage.clone();
        async move { flowr_ce::org_policy::move_sso_maps(s.as_ref()).await }
    });
    // RT-01: every FlowR reports itself, so the console sees all of them.
    flowr_ce::ops::start_host_heartbeat(storage.clone());
    if enforcing {
        flowr_ce::access::install(storage.clone());
        info!("namespace guard installed (projects enforced)");
    }

    info!("FlowR initialized successfully");

    // Task handlers (lancedb, LLM, agent, etc.) now live in flowr-ce and are
    // registered natively — no EE-side injection.

    spawn_worker(storage.clone());

    match mode.as_str() {
        "http" | "sse" => {
            restore_triggers(&storage).await;

            // Clone storage for the EE policy-admin endpoints (reload re-reads the
            // registry) before `build_ce_router` moves it.
            let sec_storage = storage.clone();
            let ce_router = build_ce_router(storage.clone());

            // Superadmin / platform seeding is handled by the a3t install flow
            // (a3t-install → a3t_schema_install), not in-process here.

            // Where `users` lives: the embedded LanceDB, or ADB (ADB stage G), looked
            // up per login so a switch from Settings takes effect at once.
            let platform = flowr_ce::platform_db::PlatformSource::Registry {
                storage: sec_storage.clone(),
                default_path: flowr_config.a3t.db_path.clone(),
            };
            info!(platform_db = %platform.get().await.describe(), "A3T platform database");
            let mut router = {
                let auth_config = Arc::new(AuthConfig {
                    platform,
                    jwt_secret: flowr_config.auth.jwt_secret.clone(),
                    token_expiry_secs: flowr_config.auth.token_expiry,
                    refresh_expiry_secs: flowr_config.auth.refresh_expiry,
                    a3t_db_path: flowr_config.a3t.db_path.clone(),
                    // Canonical external URL, used as the OAuth resource identifier.
                    resource_uri: std::env::var("MCP_RESOURCE_URI")
                        .unwrap_or_else(|_| "http://localhost:3002".to_string()),
                    // OIDC login fallback (EE): lets Keycloak users sign in via the
                    // same email/password form. Empty issuer disables it.
                    oidc_issuer: oidc_issuer.clone(),
                    oidc_client_id: oidc_client_id.clone(),
                    revoked: Arc::new(sessions::Revoked::default()),
                });
                sessions::spawn_upkeep(auth_config.clone());

                info!(
                    a3t_db = %auth_config.a3t_db_path,
                    token_expiry = auth_config.token_expiry_secs,
                    "JWT auth enabled (authorization via the AESOP policy hook)"
                );

                // Auth + OAuth discovery routes (public). Merged after the auth
                // middleware layer below, so they bypass it.
                let auth_routes = Router::new()
                    .route("/auth/login", post(login_handler))
                    .route("/auth/signup", post(signup_handler))
                    .route("/auth/me", get(me_handler))
                    // Sign-in per organisation (.ai/A3T_ORG_SSO_SPEC.md).
                    .route("/auth/sso/start", get(sso::start_handler))
                    .route("/auth/sso/callback", get(sso::callback_handler))
                    // An editor trades its handoff code for a session (sso.rs).
                    .route("/auth/sso/token", post(sso::token_handler))
                    // E0.6: report active edition + policy hook (public).
                    .route("/edition", get(edition_handler))
                    .route(
                        "/.well-known/oauth-protected-resource",
                        get(oauth_protected_resource),
                    )
                    .route(
                        "/.well-known/oauth-authorization-server",
                        get(oauth_authorization_server),
                    )
                    .layer(Extension(auth_config.clone()));

                // E2 #4: choose the AuthN layer by `security.authn`. "oidc" (EE)
                // installs a3t-security's RS256/JWKS check; otherwise the HS256 dev
                // layer. Both bypass the public routes (merged after). The audit
                // endpoint sits behind the layer.
                // EE policy-admin surface: list / hot-reload / dry-run test. All
                // InfoSec-gated (see `is_infosec`: SuperAdmin or SecurityAdmin), all behind the AuthN layer.
                let policy_admin = PolicyAdminState {
                    storage: sec_storage,
                    policies_dir: policies_dir.clone(),
                    hook_setting: hook_setting.clone(),
                };
                #[cfg(feature = "ee")]
                follow_policy_reloads(policy_admin.clone());
                // Every saved Cedar policy compiles, and applies at once on every FlowR.
                #[cfg(feature = "ee")]
                if policy_admin.hook_setting == "cedar" && policy_admin.policies_dir.is_empty() {
                    flowr_ce::policy_gate::install(Arc::new(ServerPolicyGate(policy_admin.clone())));
                }
                // EE IdP-admin surface: connect to Keycloak + create/list users.
                let idp_admin = IdpAdminState {
                    issuer: oidc_issuer.clone(),
                    admin_user: oidc_admin_user.clone(),
                    admin_password: oidc_admin_password.clone(),
                    client: reqwest::Client::new(),
                };
                // "Forgot password" (forgot.rs): public, like /auth/login.
                let forgot_routes = Router::new()
                    .route("/auth/forgot", post(forgot::forgot_handler))
                    // First sign-in with a temporary password (first_password.rs).
                    .route("/auth/first-password", post(first_password::first_password_handler))
                    // A new session from a refresh token, with roles read again (refresh.rs).
                    .route("/auth/refresh", post(refresh::refresh_handler))
                    .route("/auth/logout", post(auth_routes::logout_handler))
                    .layer(Extension(idp_admin.clone()))
                    .layer(Extension(auth_config.clone()));
                let base = ce_router
                    .merge(Router::new().route("/security/audit", get(audit_handler)).layer(Extension(storage.clone())))
                    // The permission catalogue roles and rules are built from (a3t/policies/catalogue.yaml).
                    .merge(Router::new().route("/security/catalogue", get(catalogue_handler)))
                    // Every role, in one table (ID-1): a3t/policies/roles.yaml.
                    .merge(Router::new().route("/security/roles", get(|| async { axum::Json(flowr_policy::roles::table().clone()) })))
                    // The baseline pack this build enforces (UI-5: the console shows this, not a copy).
                    .merge(Router::new().route("/security/baseline", get(baseline_handler)))
                    // Policy files that no longer compile, and what they closed (GOV-4).
                    .merge(Router::new().route("/security/policy-health", get(policy_health_handler)).layer(Extension(storage.clone())))
                    // Replay an org's policy change against its last days of decisions (GOV-2).
                    .merge({
                        #[cfg(feature = "ee")]
                        { Router::new().route("/security/orgs/:org/replay", post(replay::replay_handler)).layer(Extension(storage.clone())) }
                        #[cfg(not(feature = "ee"))]
                        { Router::new() }
                    })
                    .merge(
                        Router::new()
                            .route("/security/policies", get(policies_list_handler))
                            .route("/security/policies/reload", post(policies_reload_handler))
                            .route("/security/policies/test", post(policies_test_handler))
                            .layer(Extension(policy_admin)),
                    )
                    .merge(
                        Router::new()
                            .route("/security/idp/status", get(idp_status_handler))
                            .route("/security/idp/users", get(idp_list_users_handler).post(idp_create_user_handler))
                            .route("/security/idp/users/:id",
                                get(idp_get_user_handler).put(idp_update_user_handler).delete(idp_delete_user_handler))
                            // A SuperAdmin acting as a user or a role (impersonate.rs).
                            .route("/auth/impersonate", post(impersonate::impersonate_handler))
                            // Change your own password, proving the current one (change_password.rs).
                            .route("/auth/change-password", post(change_password::change_password_handler))
                            // People a member can be picked from, for project owners and org admins (directory.rs).
                            .route("/directory/users", get(directory::directory_users_handler))
                            // Who can reach a project, and how; everyone's access for an audit (people.rs).
                            .route("/projects/:project/people", get(people::project_people_handler))
                            .route("/security/people", get(people::security_people_handler))
                            .layer(Extension(idp_admin)),
                    );

                let secured;
                #[cfg(feature = "ee")]
                {
                    if authn == "oidc" {
                        let audience = if oidc_audience.is_empty() { None } else { Some(oidc_audience.clone()) };
                        let cfg = a3t_security::oidc::AuthnConfig::from_issuer(&oidc_issuer, audience);
                        info!(issuer = %oidc_issuer, jwks = %cfg.jwks_uri, "AuthN: OIDC (RS256/JWKS via a3t-security)");
                        let state = Arc::new(auth::OidcState {
                            cache: a3t_security::oidc::JwksCache::new(cfg.jwks_uri.clone()),
                            config: cfg,
                        });
                        secured = base
                            .layer(middleware::from_fn(auth::oidc_auth_middleware))
                            .layer(Extension(state));
                    } else if authn == "dual" {
                        // Console keeps its HS256 token; external MCP clients (Claude
                        // Desktop) present Keycloak OIDC tokens. Both are accepted.
                        let audience = if oidc_audience.is_empty() { None } else { Some(oidc_audience.clone()) };
                        let cfg = a3t_security::oidc::AuthnConfig::from_issuer(&oidc_issuer, audience);
                        info!(issuer = %oidc_issuer, jwks = %cfg.jwks_uri, "AuthN: dual (HS256 + OIDC RS256/JWKS)");
                        let state = Arc::new(auth::OidcState {
                            cache: a3t_security::oidc::JwksCache::new(cfg.jwks_uri.clone()),
                            config: cfg,
                        });
                        secured = base
                            .layer(middleware::from_fn(auth::dual_auth_middleware))
                            .layer(Extension(state))
                            .layer(Extension(auth_config));
                    } else {
                        info!("AuthN: HS256 shared-secret");
                        secured = base
                            .layer(middleware::from_fn(jwt_auth_middleware))
                            .layer(Extension(auth_config));
                    }
                }
                #[cfg(not(feature = "ee"))]
                {
                    let _ = &authn;
                    info!("AuthN: HS256 shared-secret");
                    secured = base
                        .layer(middleware::from_fn(jwt_auth_middleware))
                        .layer(Extension(auth_config));
                }
                secured.merge(auth_routes).merge(forgot_routes)
            };

            // CORS
            let cors_origins: Vec<axum::http::HeaderValue> = flowr_config.cors.origins.iter()
                .filter_map(|s| s.parse().ok())
                .collect();
            info!(origins = ?flowr_config.cors.origins, "CORS enabled");

            let cors = tower_http::cors::CorsLayer::new()
                .allow_origin(cors_origins)
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::PUT,
                    axum::http::Method::DELETE,
                    axum::http::Method::OPTIONS,
                ])
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::AUTHORIZATION,
                ])
                .allow_credentials(true);

            router = router.layer(cors);

            serve(router, port).await?;
        }
        _ => {
            run_stdio(storage).await?;
        }
    }

    info!("FlowR shutting down");
    Ok(())
}

fn get_arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

// =============================================================================
// OAuth 2.0 discovery metadata (MCP authorization)
// =============================================================================

fn resource_uri() -> String {
    std::env::var("MCP_RESOURCE_URI").unwrap_or_else(|_| "http://localhost:3002".to_string())
}

/// Cached edition/policy-hook info, populated once at startup for `GET /edition`.
static EDITION_INFO: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();

/// E0.6 — install the AESOP policy hook named by `security.policy_hook` and
/// return the *effective* hook name (may differ from the request if the named
/// hook isn't compiled into this build). AllowAll is the default when unset, so
/// "allow_all" needs no install; `policy_hook()` already returns it.
/// E2 #2c: every InfoSec-owned Cedar policy resource (`type_ref: cedar`) in the
/// registry, in every namespace, as (namespace, name, `config.cedar`). Those in
/// system / a3t / default are the platform's; any other is its project's
/// (a3t_security::NamedPolicy). What is stored compiles: the policy gate checks
/// each one before it is saved.
pub(crate) async fn collect_registry_cedar_policies(
    storage: &Arc<dyn flowr_ce::flowr_storage::StorageBackend>,
) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for ns in &flowr_ce::registry::resource_namespaces(storage.as_ref()).await {
        if let Ok(resources) = storage.resources().list(ns).await {
            for r in resources {
                if r.type_ref == "cedar" {
                    if let Some(src) = r.config.get("cedar").and_then(|v| v.as_str()) {
                        out.push((r.namespace.clone(), r.name.clone(), src.to_string()));
                    }
                }
            }
        }
    }
    // Each agent's own security policy. It goes in the platform set: an agent's
    // turns run through A3T's agent flow, wherever the agent lives, and every
    // statement names the agent, so it binds that agent only. It was checked
    // when saved; one that no longer compiles (the catalogue changed) is
    // reported and left out, and the rest of the set still loads.
    // GOV-4: one that no longer compiles is never quietly left out — what it
    // guarded is closed, naming the file, until a security admin fixes it.
    #[cfg(feature = "ee")]
    let mut broken: Vec<flowr_ce::policy_health::Broken> = Vec::new();
    #[cfg(feature = "ee")]
    let now = chrono::Utc::now().to_rfc3339();
    #[cfg(feature = "ee")]
    for (id, project, yaml) in flowr_ce::agent_space::security::all_policies(storage.as_ref()).await {
        match a3t_security::policy_files::compile_agent_policy(&project, &id, &yaml) {
            Ok(c) => out.push(("a3t".to_string(), format!("agent-{id}"), c.cedar)),
            Err(e) => {
                let why = format!("Agent {id}'s own policy no longer compiles, so the agent is stopped until an org admin or security admin fixes it (Security › the agent › Security). {e}");
                out.push(("a3t".to_string(), format!("agent-{id}-broken"),
                    flowr_ce::policy_health::closing_agent_statement(&format!("agent.{id}.broken"), &id, &why)));
                broken.push(flowr_ce::policy_health::Broken { kind: "agent".into(), owner: id.clone(), org: None,
                    errors: vec![e], closed: vec![format!("agent {id}")], since: now.clone() });
            }
        }
    }
    // Each org's files, kept in A3T: placed in the policy set of every project
    // they apply to, never outside the org. They were checked when saved; a set
    // that no longer compiles (the catalogue changed) closes the org's projects.
    #[cfg(feature = "ee")]
    let shadows = flowr_ce::org_policy::all_shadow_files(storage.as_ref()).await;
    // Every org with enforced files, and every org with shadow files only.
    #[cfg(feature = "ee")]
    let mut orgs = flowr_ce::org_policy::all_org_files(storage.as_ref()).await;
    #[cfg(feature = "ee")]
    for org in shadows.keys() {
        if !orgs.iter().any(|(o, _, _)| o == org) {
            orgs.push((org.clone(), flowr_ce::org_policy::org_projects(storage.as_ref(), org).await, Vec::new()));
        }
    }
    #[cfg(feature = "ee")]
    for (org, projects, files) in orgs {
        let files: Vec<a3t_security::policy_files::OrgFile> = files.into_iter()
            .map(|(path, content)| a3t_security::policy_files::OrgFile { path, content }).collect();
        // GOV-2: its shadow files, compiled with the enforced ones (they may name
        // the org's roles), placed under `shadow:` names: evaluated, not enforced.
        if let Some(sh) = shadows.get(&org) {
            let mut all = files.clone();
            all.extend(sh.iter().map(|(path, content)| a3t_security::policy_files::OrgFile { path: path.clone(), content: content.clone() }));
            match a3t_security::policy_files::compile_org_set(&org, &projects, &all) {
                Ok(set) => {
                    let shadow_labels: std::collections::BTreeSet<String> = set.files.iter()
                        .filter(|f| sh.iter().any(|(p, _)| f["path"].as_str() == Some(p)))
                        .filter_map(|f| Some(format!("org.{org}.{}.{}", f["kind"].as_str()?, f["name"].as_str()?.rsplit('/').next()?)))
                        .collect();
                    for (p, label, cedar) in set.policies {
                        if shadow_labels.contains(&label) {
                            out.push((p, format!("{}{label}", a3t_security::SHADOW_PREFIX), cedar));
                        }
                    }
                }
                Err(errors) => warn!(org = %org, errors = ?errors, "an org's shadow files don't compile with its enforced ones; they are not evaluated"),
            }
        }
        if files.is_empty() {
            continue; // shadow files only: placed above
        }
        match a3t_security::policy_files::compile_org_set(&org, &projects, &files) {
            Ok(set) => out.extend(set.policies),
            Err(errors) => {
                let why = format!("{org}'s policy files no longer compile, so its projects refuse every call until an org admin or security admin fixes them (Security › {org} › Policy files): {}", errors.join("; "));
                for p in &projects {
                    out.push((p.clone(), format!("org-{org}-broken"),
                        flowr_ce::policy_health::closing_statement(&format!("org.{org}.broken"), &why)));
                }
                broken.push(flowr_ce::policy_health::Broken { kind: "org".into(), owner: org.clone(), org: Some(org.clone()),
                    errors, closed: projects.clone(), since: now.clone() });
            }
        }
    }
    #[cfg(feature = "ee")]
    {
        let (fresh, healed) = flowr_ce::policy_health::set(broken);
        flowr_ce::policy_health::log(&fresh, &healed);
    }
    out
}

#[cfg(feature = "ee")]
pub(crate) fn named(policies: &[(String, String, String)]) -> Vec<a3t_security::NamedPolicy> {
    policies.iter().map(|(ns, n, s)| a3t_security::NamedPolicy::new(ns.clone(), n.clone(), s.clone())).collect()
}

/// Collect the registry-supplied Cedar **schema**: an optional base override
/// (`type_ref: cedarschema`, name `base-schema`) plus per-resource fragments
/// (a `type_ref: cedarschema` resource's `config.schema`, or any resource's
/// inline `config.cedar_schema`). Values may be a JSON object or a JSON string.
pub(crate) async fn collect_registry_schema(
    storage: &Arc<dyn flowr_ce::flowr_storage::StorageBackend>,
) -> (Option<String>, Vec<String>) {
    let as_str = |v: &serde_json::Value| -> String {
        v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string())
    };
    let mut base_override: Option<String> = None;
    let mut fragments = Vec::new();
    for ns in &flowr_ce::registry::resource_namespaces(storage.as_ref()).await {
        if let Ok(resources) = storage.resources().list(ns).await {
            for r in resources {
                if r.type_ref == "cedarschema" {
                    if let Some(s) = r.config.get("schema") {
                        if r.name == "base-schema" {
                            base_override = Some(as_str(s));
                        } else {
                            fragments.push(as_str(s));
                        }
                    }
                    continue;
                }
                if let Some(s) = r.config.get("cedar_schema") {
                    fragments.push(as_str(s));
                }
            }
        }
    }
    (base_override, fragments)
}

fn install_policy_hook(
    name: &str,
    policies_dir: &str,
    registry_policies: &[(String, String, String)],
    schema_base: Option<&str>,
    schema_fragments: &[String],
) -> &'static str {
    let _ = policies_dir; // used only by the ee build
    let _ = registry_policies; // used only by the ee build
    let _ = schema_base; // used only by the ee build
    let _ = schema_fragments; // used only by the ee build
    match name {
        "deny_all" => {
            let _ = flowr_policy::set_policy_hook(Arc::new(flowr_policy::DenyAll));
            "deny_all"
        }
        "cedar" => {
            #[cfg(feature = "ee")]
            {
                // EE build: install the Cedar hook. Load order (E2 #2c, §6.3):
                //   • security.policies_dir set  → file-based pack (deploy override)
                //   • otherwise                  → baked baseline + attached registry
                //     `type_ref: cedar` policy resources owned by InfoSec.
                let loaded = if !policies_dir.is_empty() {
                    a3t_security::CedarHook::from_dir(policies_dir)
                } else {
                    a3t_security::CedarHook::with_named_policies(
                        &named(registry_policies),
                        schema_base,
                        schema_fragments,
                    )
                };
                match loaded {
                    Ok(hook) => {
                        let _ = flowr_policy::set_policy_hook(Arc::new(hook));
                        info!(policies_dir = %policies_dir, registry_policies = registry_policies.len(), "AESOP Cedar policy hook installed (enforcing)");
                        return "cedar";
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "cedar hook failed to load; failing closed to deny_all");
                        let _ = flowr_policy::set_policy_hook(Arc::new(flowr_policy::DenyAll));
                        return "deny_all";
                    }
                }
            }
            #[cfg(not(feature = "ee"))]
            {
                // CE build has no Cedar hook — fall back to the permissive default.
                warn!(
                    "security.policy_hook='cedar' requested, but the Cedar hook is \
                     EE-only and not compiled into this build; falling back to allow_all"
                );
                "allow_all"
            }
        }
        _ => "allow_all",
    }
}

/// `GET /security/audit` — the decision log (AUD-1/5/8).
///
/// Read from the durable log (ADB EPISODIC, `_audit.<org>`), filtered and paged
/// on the server: `org`, `project`, `agent`, `principal`, `action`, `decision`
/// (exact values), `since_ms`, `before_ms` (the next page: the last row's
/// `ts_ms`), `q` (ranked by meaning), `limit` (≤ 500, default 100).
///
/// Who sees what:
/// - SuperAdmin / SecurityAdmin: every org (or the `org` named);
/// - an org's admins and security admins: that org (`org` required);
/// - anyone else: decisions about the agents they own.
///
/// Without the durable log (CE), this pod's recent decisions, for InfoSec only.
async fn audit_handler(
    principal: Option<axum::Extension<flowr_policy::Principal>>,
    storage: Option<axum::Extension<Arc<dyn StorageBackend>>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    let refuse = |code: StatusCode, msg: &str| (code, axum::Json(serde_json::json!({ "error": msg }))).into_response();
    let Some(axum::Extension(p)) = principal else { return refuse(StatusCode::UNAUTHORIZED, "Sign in first.") };
    let infosec = is_infosec(Some(&p));
    let param = |k: &str| params.get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let num = |k: &str| param(k).and_then(|v| v.parse::<u64>().ok());
    let limit: usize = num("limit").map(|n| n as usize).unwrap_or(100).clamp(1, 500);

    if !flowr_ce::decision_log::installed() {
        if !infosec {
            return refuse(StatusCode::FORBIDDEN, "The decision log is for SuperAdmin and SecurityAdmin.");
        }
        let project = param("project");
        let mut records: Vec<_> = flowr_policy::audit::recent(1000).into_iter()
            .filter(|r| project.as_deref().is_none_or(|p| r.project.as_deref() == Some(p)))
            .filter(|r| param("decision").as_deref().is_none_or(|d| r.decision == d))
            .collect();
        records.truncate(limit);
        return axum::Json(serde_json::json!({ "count": records.len(), "records": records, "scope": "this FlowR replica, in memory (newest 1000)" })).into_response();
    }

    let mut q = flowr_ce::decision_log::Query {
        project: param("project"), agent: param("agent"), principal: param("principal"),
        action: param("action"), decision: param("decision"),
        since_ms: num("since_ms"), before_ms: num("before_ms"), like: param("q"), limit,
        ..Default::default()
    };
    let every_org = || async {
        let mut o = flowr_ce::access::all_org_ids().await;
        o.push("platform".to_string());
        o
    };
    let scope = if infosec {
        q.orgs = match param("org") { Some(o) => vec![o], None => every_org().await };
        "all"
    } else {
        let manages = match (param("org"), storage.as_ref()) {
            (Some(org), Some(axum::Extension(s))) => flowr_ce::org_policy::manages_org_security(s.as_ref(), &p, &org).await.then_some(org),
            _ => None,
        };
        match manages {
            Some(org) => { q.orgs = vec![org]; "org" }
            None => {
                // Decisions about the agents this person owns, wherever they ran.
                let Some(me) = flowr_policy::namespace::acting_user(&p) else {
                    return refuse(StatusCode::FORBIDDEN, "The decision log is read by people, not agents.");
                };
                q.orgs = every_org().await;
                // Or what they did themselves (their approval answers, GOV-7).
                if q.principal.as_deref() == Some(me) {
                    "yours"
                } else {
                    q.agent_owner = Some(me.to_string());
                    "own agents"
                }
            }
        }
    };
    match flowr_ce::decision_log::recall(&q).await {
        Ok((records, aql)) => {
            let next = (records.len() >= limit && q.like.is_none()).then(|| records.last().and_then(|r| r["ts_ms"].as_u64())).flatten();
            axum::Json(serde_json::json!({
                "count": records.len(), "records": records, "scope": scope, "orgs": q.orgs,
                "next_before_ms": next, "aql": aql, "source": "ADB EPISODIC (durable, every pod)",
            })).into_response()
        }
        Err(e) => {
            warn!(error = %e, "decision log: read failed");
            refuse(StatusCode::SERVICE_UNAVAILABLE, "The decision log can't be read right now. Try again in a moment.")
        }
    }
}

/// `GET /edition` — reports the active edition and policy hook (public).
async fn edition_handler() -> impl IntoResponse {
    axum::Json(EDITION_INFO.get().cloned().unwrap_or_else(|| {
        serde_json::json!({ "edition": "ce", "policy_hook": "allow_all" })
    }))
}

// =============================================================================
// EE policy administration — list / hot-reload / dry-run test
//   POST /security/policies/reload  → limitation #1 (no more pod restart)
//   POST /security/policies/test    → limitation #4 (test before you commit)
//   GET  /security/policies         → limitation #2/#3 (a real Policies surface)
// The write/apply paths (create/update/delete a cedar resource) reuse the
// existing `/registry/resources` CRUD; these add the *operate* surface on top.
// =============================================================================

/// State for the policy-admin endpoints: storage (to re-read `type_ref: cedar`
/// registry resources) plus the hook config needed to rebuild on reload.
#[derive(Clone)]
struct PolicyAdminState {
    storage: Arc<dyn flowr_ce::flowr_storage::StorageBackend>,
    policies_dir: String,
    hook_setting: String,
}

/// `GET /security/baseline` — the baseline policy pack compiled into this build,
/// and its version (sha256, first 16 hex). Anyone signed in may read it.
async fn baseline_handler() -> axum::response::Response {
    #[cfg(feature = "ee")]
    {
        use sha2::Digest;
        let cedar = a3t_security::hook::A3T_BASELINE;
        let version: String = sha2::Sha256::digest(cedar.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect();
        axum::Json(serde_json::json!({ "cedar": cedar, "version": version })).into_response()
    }
    #[cfg(not(feature = "ee"))]
    {
        (axum::http::StatusCode::NOT_FOUND, axum::Json(serde_json::json!({ "error": "A Community runtime has no baseline: it enforces nothing." }))).into_response()
    }
}

/// `GET /security/policy-health` — policy files that no longer compile and
/// what they closed (GOV-4). InfoSec sees all; an org's admins and security
/// admins see their org's.
async fn policy_health_handler(
    principal: Option<axum::Extension<flowr_policy::Principal>>,
    storage: Option<axum::Extension<Arc<dyn StorageBackend>>>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    let Some(axum::Extension(p)) = principal else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "Sign in first." }))).into_response();
    };
    let infosec = is_infosec(Some(&p));
    let mut shown = Vec::new();
    for b in flowr_ce::policy_health::list() {
        let ok = infosec || match (b.org.as_deref(), storage.as_ref()) {
            (Some(org), Some(axum::Extension(s))) => flowr_ce::org_policy::manages_org_security(s.as_ref(), &p, org).await,
            _ => false,
        };
        if ok { shown.push(b); }
    }
    axum::Json(serde_json::json!({ "broken": shown, "count": shown.len() })).into_response()
}

/// `GET /security/catalogue` — every permission a role or rule can name, with
/// what enforces it. Not a secret: anyone signed in may read it (the console
/// builds its role and rule editors from it).
async fn catalogue_handler() -> impl IntoResponse {
    axum::Json(flowr_policy::catalogue::catalogue().clone())
}

/// Policy administration is restricted to InfoSec/admin roles. Gate on the
/// canonical roles carried on the authenticated Principal (built by the AuthN
/// layer via `roles::canonicalize`). Deliberately does NOT route through the
/// Cedar hook: reload must stay reachable even if a bad policy set turned the
/// hook deny-all, so an admin can always recover.
/// Platform InfoSec: SuperAdmin or SecurityAdmin. Security (policies, identity)
/// and the decision log are theirs alone — an Admin runs projects, not security.
fn is_infosec(principal: Option<&flowr_policy::Principal>) -> bool {
    use flowr_policy::{roles, Principal};
    matches!(principal, Some(Principal::Human { roles: r, .. })
        if r.iter().any(|role| role == roles::SUPER_ADMIN || role == roles::SECURITY_ADMIN))
}

/// `GET /security/policies` — list InfoSec-owned Cedar policy resources
/// (`type_ref == "cedar"`) with their source, so the UI has a first-class
/// Policies view instead of the generic resource form. SuperAdmin and
/// SecurityAdmin see every policy; an org's admins and security admins see
/// those of the org's projects (reading them; changing them stays InfoSec's).
async fn policies_list_handler(
    Extension(state): Extension<PolicyAdminState>,
    principal: Option<Extension<flowr_policy::Principal>>,
) -> impl IntoResponse {
    let p = principal.as_ref().map(|e| &e.0);
    // None: every namespace; Some: only these.
    let scope: Option<std::collections::HashSet<String>> = if is_infosec(p) {
        None
    } else {
        let user = p.and_then(flowr_policy::namespace::acting_user).map(String::from);
        let snap = flowr_ce::access::current_snapshot().await;
        let orgs = match (&user, &snap) { (Some(u), Some(s)) => s.security_orgs_of(u), _ => Vec::new() };
        if orgs.is_empty() {
            return (axum::http::StatusCode::FORBIDDEN,
                axum::Json(serde_json::json!({ "error": "SuperAdmin, SecurityAdmin or an org's admin or security admin role required" }))).into_response();
        }
        let snap = snap.expect("orgs come from the snapshot");
        Some(orgs.iter().flat_map(|h| snap.org_namespaces(h)).collect())
    };
    let mut policies = Vec::new();
    for (ns, name, _) in collect_registry_cedar_policies(&state.storage).await {
        if scope.as_ref().is_some_and(|s| !s.contains(&ns)) {
            continue;
        }
        if let Ok(r) = state.storage.resources().get(&ns, &name).await {
            let platform = ["system", "a3t", "default"].contains(&ns.as_str());
            policies.push(serde_json::json!({
                "namespace": r.namespace,
                "name": r.name,
                "cedar": r.config.get("cedar").and_then(|v| v.as_str()).unwrap_or(""),
                "description": r.config.get("description").and_then(|v| v.as_str()),
                // Where it applies: every project (platform), or its own.
                "scope": if platform { "platform".to_string() } else { format!("project {ns}") },
            }));
        }
    }
    axum::Json(serde_json::json!({ "count": policies.len(), "policies": policies })).into_response()
}

/// Engine topic announcing a policy reload to every FlowR in the cluster.
const POLICY_TOPIC: &str = "cluster:policies";

/// Re-read the registry's Cedar policies and schema and swap the hook in if
/// they load cleanly. Returns (policies, schema fragments).
#[cfg(feature = "ee")]
async fn reload_policies(state: &PolicyAdminState) -> Result<(usize, usize), String> {
    let policies = collect_registry_cedar_policies(&state.storage).await;
    let count = policies.len();
    let (schema_base, schema_fragments) = collect_registry_schema(&state.storage).await;
    let frag_count = schema_fragments.len();
    let loaded = if !state.policies_dir.is_empty() {
        a3t_security::CedarHook::from_dir(&state.policies_dir)
    } else {
        a3t_security::CedarHook::with_named_policies(&named(&policies), schema_base.as_deref(), &schema_fragments)
    };
    match loaded {
        Ok(hook) => {
            let _ = flowr_policy::set_policy_hook(Arc::new(hook));
            info!(registry_policies = count, schema_fragments = frag_count, "policy hot-reload applied");
            Ok((count, frag_count))
        }
        Err(e) => {
            warn!(error = %e, "policy hot-reload rejected; keeping current hook");
            Err(e.to_string())
        }
    }
}

/// The registry's policy gate (flowr_ce::policy_gate): a policy is saved only if
/// the whole set still compiles with it, and the set is reloaded here and on
/// every other FlowR after it is saved or deleted.
#[cfg(feature = "ee")]
struct ServerPolicyGate(PolicyAdminState);

#[cfg(feature = "ee")]
#[async_trait::async_trait]
impl flowr_ce::policy_gate::PolicyGate for ServerPolicyGate {
    async fn check(&self, namespace: &str, name: &str, source: Option<&str>) -> Result<(), String> {
        let mut policies: Vec<(String, String, String)> = collect_registry_cedar_policies(&self.0.storage)
            .await
            .into_iter()
            .filter(|(ns, n, _)| !(ns == namespace && n == name))
            .collect();
        if let Some(src) = source {
            if src.trim().is_empty() {
                return Err(format!("Policy {namespace}/{name} is empty: write at least one permit or forbid."));
            }
            policies.push((namespace.to_string(), name.to_string(), src.to_string()));
        }
        let (schema_base, schema_fragments) = collect_registry_schema(&self.0.storage).await;
        a3t_security::CedarHook::validate(&named(&policies), schema_base.as_deref(), &schema_fragments)
            .map_err(|e| format!("Policy {namespace}/{name} was not saved: it does not compile. {e}"))
    }

    async fn compile_agent_policy(&self, project: &str, agent_id: &str, yaml: &str) -> Result<serde_json::Value, String> {
        let c = a3t_security::policy_files::compile_agent_policy(project, agent_id, yaml)?;
        // It must also load with everything else that is saved.
        let mut policies: Vec<(String, String, String)> = collect_registry_cedar_policies(&self.0.storage)
            .await.into_iter().filter(|(_, n, _)| n != &format!("agent-{agent_id}")).collect();
        policies.push(("a3t".to_string(), format!("agent-{agent_id}"), c.cedar.clone()));
        let (schema_base, schema_fragments) = collect_registry_schema(&self.0.storage).await;
        a3t_security::CedarHook::validate(&named(&policies), schema_base.as_deref(), &schema_fragments)
            .map_err(|e| format!("The policy was not saved: it does not load with the others. {e}"))?;
        Ok(serde_json::json!({ "name": c.name, "placement": c.placement, "hidden_apps": c.hidden_apps }))
    }

    async fn compile_org_files(&self, org: &str, projects: &[String], files: &[(String, String)]) -> Result<serde_json::Value, Vec<String>> {
        let files: Vec<a3t_security::policy_files::OrgFile> = files.iter()
            .map(|(path, content)| a3t_security::policy_files::OrgFile { path: path.clone(), content: content.clone() }).collect();
        let set = a3t_security::policy_files::compile_org_set(org, projects, &files)?;
        // It must also load with everything else: the other orgs, the agents, the platform.
        let mine = format!("org.{org}.");
        let mut policies: Vec<(String, String, String)> = collect_registry_cedar_policies(&self.0.storage)
            .await.into_iter().filter(|(_, n, _)| !n.starts_with(&mine)).collect();
        policies.extend(set.policies.iter().cloned());
        let (schema_base, schema_fragments) = collect_registry_schema(&self.0.storage).await;
        a3t_security::CedarHook::validate(&named(&policies), schema_base.as_deref(), &schema_fragments)
            .map_err(|e| vec![format!("the files do not load with the other policies: {e}")])?;
        Ok(serde_json::json!({ "files": set.files, "hidden_apps": set.hidden_apps, "access": set.access }))
    }

    async fn changed(&self) {
        match reload_policies(&self.0).await {
            Ok(_) => {
                if let Some(engine) = self.0.storage.engine() {
                    let from = std::env::var("HOSTNAME").unwrap_or_default();
                    if let Err(e) = engine.publish(POLICY_TOPIC, serde_json::json!({ "from": from })).await {
                        warn!(error = %e, "policy change not announced to the cluster");
                    }
                }
            }
            Err(e) => warn!(error = %e, "a saved policy change did not load"),
        }
    }
}

/// Cluster: reload when another FlowR announces a policy reload.
#[cfg(feature = "ee")]
fn follow_policy_reloads(state: PolicyAdminState) {
    use futures::StreamExt;
    if state.hook_setting != "cedar" {
        return;
    }
    let Some(engine) = state.storage.engine() else { return };
    let mut notices = engine.subscribe(POLICY_TOPIC);
    tokio::spawn(async move {
        while let Some(n) = notices.next().await {
            let me = std::env::var("HOSTNAME").unwrap_or_default();
            if n.get("from").and_then(|f| f.as_str()) == Some(me.as_str()) && !me.is_empty() {
                continue;
            }
            let _ = reload_policies(&state).await;
        }
    });
}

/// `POST /security/policies/reload` — hot-reload the Cedar policy set with no
/// process restart (limitation #1). Re-reads the registry, rebuilds the hook,
/// and **only swaps it in if it loads cleanly** — a malformed draft leaves the
/// currently-enforcing hook untouched rather than bricking enforcement.
async fn policies_reload_handler(
    Extension(state): Extension<PolicyAdminState>,
    principal: Option<Extension<flowr_policy::Principal>>,
) -> impl IntoResponse {
    if !is_infosec(principal.as_ref().map(|e| &e.0)) {
        return (axum::http::StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "error": "SuperAdmin or SecurityAdmin role required" }))).into_response();
    }
    if state.hook_setting != "cedar" {
        return axum::Json(serde_json::json!({
            "reloaded": false,
            "reason": format!("policy_hook is '{}', not cedar — nothing to reload", state.hook_setting),
        })).into_response();
    }
    #[cfg(feature = "ee")]
    {
        match reload_policies(&state).await {
            Ok((count, frag_count)) => {
                // Every other FlowR in the cluster reloads too.
                if let Some(engine) = state.storage.engine() {
                    let from = std::env::var("HOSTNAME").unwrap_or_default();
                    if let Err(e) = engine.publish(POLICY_TOPIC, serde_json::json!({ "from": from })).await {
                        warn!(error = %e, "policy reload not announced to the cluster");
                    }
                }
                axum::Json(serde_json::json!({ "reloaded": true, "registry_policies": count, "schema_fragments": frag_count })).into_response()
            }
            Err(e) => {
                // Do NOT swap — keep the working hook so a bad policy can't brick
                // enforcement. Surface the load error for the UI to display.
                (axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    axum::Json(serde_json::json!({ "reloaded": false, "error": e }))).into_response()
            }
        }
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = &state;
        (axum::http::StatusCode::NOT_IMPLEMENTED,
            axum::Json(serde_json::json!({ "error": "policy reload is an EE feature" }))).into_response()
    }
}

/// Body for `POST /security/policies/test`.
#[derive(serde::Deserialize)]
struct PolicyTestRequest {
    /// Who is acting (full `flowr_policy::Principal`, incl. roles).
    principal: flowr_policy::Principal,
    /// `domain:verb`, e.g. `tool:invoke`, `resource:create`.
    action: String,
    /// The target entity.
    resource: PolicyTestResource,
    /// Ambient principal attributes the policy may read — notably
    /// `{"template": {"exposedTools": [...]}}` to exercise the flow ceiling.
    #[serde(default)]
    attributes: serde_json::Map<String, serde_json::Value>,
    /// Uncommitted Cedar to evaluate *before* saving it (the "test before you
    /// commit" path). Layered on top of the baseline pack.
    #[serde(default)]
    draft_policies: Vec<String>,
    /// Whether to also include the currently-saved registry policies (default
    /// true). Set false to test a draft against the baseline in isolation.
    #[serde(default = "default_true")]
    include_registry: bool,
    /// The project the draft would be saved in (its calls are what it governs);
    /// none: a platform policy.
    #[serde(default)]
    draft_namespace: Option<String>,
}

#[derive(serde::Deserialize)]
struct PolicyTestResource {
    #[serde(rename = "type")]
    type_name: String,
    id: String,
    #[serde(default)]
    attributes: serde_json::Map<String, serde_json::Value>,
}

fn default_true() -> bool { true }

/// `POST /security/policies/test` — evaluate a hypothetical request against the
/// baseline + (optionally) the live registry + an uncommitted draft, and return
/// the decision with matched policy ids — **without mutating the live hook**
/// (limitation #4). This is the "would this be allowed, and why?" simulator.
async fn policies_test_handler(
    Extension(state): Extension<PolicyAdminState>,
    principal: Option<Extension<flowr_policy::Principal>>,
    axum::Json(body): axum::Json<PolicyTestRequest>,
) -> impl IntoResponse {
    if !is_infosec(principal.as_ref().map(|e| &e.0)) {
        return (axum::http::StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "error": "SuperAdmin or SecurityAdmin role required" }))).into_response();
    }
    #[cfg(feature = "ee")]
    {
        let action = match flowr_policy::Action::new(&body.action) {
            Ok(a) => a,
            Err(e) => return (axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({ "error": format!("invalid action: {e}") }))).into_response(),
        };
        // Assemble the policy set: baseline (always, inside with_registry_policies)
        // + live registry (optional) + the caller's draft.
        let mut policies = if body.include_registry {
            collect_registry_cedar_policies(&state.storage).await
        } else {
            Vec::new()
        };
        // A draft is tried where it would be saved: its project, or the platform.
        let draft_ns = body.draft_namespace.clone().unwrap_or_else(|| "default".into());
        for (i, d) in body.draft_policies.iter().enumerate() {
            policies.push((draft_ns.clone(), format!("draft-{i}"), d.clone()));
        }
        // Validate/evaluate the draft against the same merged schema (base + fragments).
        let (schema_base, schema_fragments) = collect_registry_schema(&state.storage).await;
        let hook = match a3t_security::CedarHook::with_named_policies(&named(&policies), schema_base.as_deref(), &schema_fragments) {
            Ok(h) => h,
            Err(e) => return (axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                axum::Json(serde_json::json!({ "error": format!("policy set failed to load: {e}") }))).into_response(),
        };
        // Faithful simulation: for a `Tool` resource, auto-attach the same
        // security metadata the real choke point does via `Resource::tool()` —
        // `name` (from the id) and `destructive` — unless the caller already set
        // them. So testing "delete_flow" fires the destructive guardrail without
        // the caller needing to know that attribute exists.
        let mut resource = flowr_policy::Resource {
            type_name: body.resource.type_name,
            id: body.resource.id,
            attributes: body.resource.attributes,
        };
        if resource.type_name == "Tool" {
            resource.attributes.entry("name".to_string())
                .or_insert_with(|| serde_json::Value::String(resource.id.clone()));
            resource.attributes.entry("destructive".to_string())
                .or_insert_with(|| serde_json::Value::Bool(flowr_policy::is_destructive(&resource.id)));
        }
        let req = flowr_policy::ExecutionRequest {
            principal: body.principal,
            action,
            resource,
            context: Default::default(),
            attributes: body.attributes,
            params: serde_json::Value::Null,
        };
        let (decision, deciding) = hook.check_explained(&req).await;
        // This endpoint exists to show what the policy WOULD do, so `Ask` is
        // reported as itself rather than folded into a deny. Collapsing it here
        // would make the Policies page claim a request is refused when in fact
        // the user would be prompted and could allow it — the opposite of what a
        // dry run is for. `allowed` stays false (an unanswered question is not
        // permission) and `scopes` says what the prompt would offer.
        let (allowed, reason, policy_ids, scopes) = match &decision {
            flowr_policy::PolicyDecision::Allow => (true, None, deciding.clone(), Vec::new()),
            flowr_policy::PolicyDecision::Deny { reason, policy_ids } =>
                (false, Some(reason.clone()), policy_ids.clone(), Vec::new()),
            flowr_policy::PolicyDecision::Ask { reason, policy_ids, scopes, .. } => (
                false,
                Some(reason.clone()),
                policy_ids.clone(),
                scopes.iter().map(|s| s.as_str().to_string()).collect(),
            ),
        };
        axum::Json(serde_json::json!({
            "decision": decision.label(),
            "allowed": allowed,
            "reason": reason,
            "policy_ids": policy_ids,
            "scopes": scopes,
            "tested_policies": policies.len(),
        })).into_response()
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = (&state, &body);
        (axum::http::StatusCode::NOT_IMPLEMENTED,
            axum::Json(serde_json::json!({ "error": "policy test is an EE feature" }))).into_response()
    }
}

// =============================================================================
// EE IdP administration — connect to Keycloak + create/list users (limitation #5)
//   GET  /security/idp/status  → is the admin connection configured + reachable?
//   GET  /security/idp/users   → list realm users
//   POST /security/idp/users   → create a user (+ assign realm roles)
// Auth to Keycloak uses the master-realm admin (admin-cli password grant); the
// password is env-only (FLOWR_SECURITY_OIDC_ADMIN_PASSWORD). Prod should swap in
// a scoped service-account client — only a3t_security::oidc_admin::admin_token changes.
// =============================================================================

/// State for the IdP-admin endpoints. Plain strings so it compiles in CE too;
/// the ee handlers build `a3t_security::oidc_admin::IdpAdminConfig` from them.
#[derive(Clone)]
pub(crate) struct IdpAdminState {
    pub(crate) issuer: String,
    pub(crate) admin_user: String,
    pub(crate) admin_password: String,
    pub(crate) client: reqwest::Client,
}

/// Body for `POST /security/idp/users` (kept local so it exists in CE builds).
#[derive(serde::Deserialize)]
struct IdpNewUser {
    username: String,
    #[serde(default)]
    email: Option<String>,
    password: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    temporary: bool,
    #[serde(default)]
    roles: Vec<String>,
}

/// Realm roles a person may hold: the platform roles of the role table (ID-1).
fn idp_role_names() -> Vec<&'static str> {
    flowr_policy::roles::table().roles.iter().filter(|r| r.scope == "platform").map(|r| r.id.as_str()).collect()
}

fn is_super_admin(principal: Option<&flowr_policy::Principal>) -> bool {
    matches!(principal, Some(flowr_policy::Principal::Human { roles: r, .. })
        if r.iter().any(|role| role == flowr_policy::roles::SUPER_ADMIN))
}

/// Does this set of realm roles carry SuperAdmin or SecurityAdmin?
fn grants_infosec(roles: &[String]) -> bool {
    use flowr_policy::roles;
    roles.iter().flat_map(|r| roles::canonicalize(r))
        .any(|r| r == roles::SUPER_ADMIN || r == roles::SECURITY_ADMIN)
}

/// Roles one person may give another. Only known roles; and only a SuperAdmin
/// gives SuperAdmin or SecurityAdmin, so a SecurityAdmin can't raise anyone
/// (themselves included) above their own role.
fn check_role_grant(principal: Option<&flowr_policy::Principal>, roles: &[String]) -> Result<(), String> {
    let names = idp_role_names();
    if let Some(bad) = roles.iter().find(|r| !names.contains(&r.trim().to_ascii_lowercase().as_str())) {
        return Err(format!("Unknown role '{bad}'. Roles are: {}.", names.join(", ")));
    }
    if grants_infosec(roles) && !is_super_admin(principal) {
        return Err("Only a SuperAdmin can give the SuperAdmin or SecurityAdmin role.".into());
    }
    Ok(())
}

fn idp_refuse(status: axum::http::StatusCode, msg: impl Into<String>) -> axum::response::Response {
    (status, axum::Json(serde_json::json!({ "error": msg.into() }))).into_response()
}

/// A SuperAdmin or SecurityAdmin account is changed (password, roles, enabled,
/// delete) only by a SuperAdmin; otherwise a SecurityAdmin could take one over.
#[cfg(feature = "ee")]
async fn guard_infosec_target(
    state: &IdpAdminState,
    cfg: &a3t_security::oidc_admin::IdpAdminConfig,
    token: &str,
    principal: Option<&flowr_policy::Principal>,
    id: &str,
) -> Result<(), axum::response::Response> {
    if is_super_admin(principal) { return Ok(()); }
    match a3t_security::oidc_admin::user_realm_roles(&state.client, cfg, token, id).await {
        Ok(current) if grants_infosec(&current) => Err(idp_refuse(axum::http::StatusCode::FORBIDDEN,
            "Only a SuperAdmin can change a SuperAdmin or SecurityAdmin account.")),
        Ok(_) => Ok(()),
        // Not knowing who the target is must not let the change through.
        Err(e) => Err(idp_refuse(axum::http::StatusCode::BAD_GATEWAY, e.to_string())),
    }
}

/// `GET /security/idp/status` — is Keycloak admin configured, and can we reach it?
async fn idp_status_handler(
    Extension(state): Extension<IdpAdminState>,
    principal: Option<Extension<flowr_policy::Principal>>,
) -> impl IntoResponse {
    if !is_infosec(principal.as_ref().map(|e| &e.0)) {
        return (axum::http::StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "error": "SuperAdmin or SecurityAdmin role required" }))).into_response();
    }
    #[cfg(feature = "ee")]
    {
        match a3t_security::oidc_admin::IdpAdminConfig::from_issuer(
            &state.issuer, state.admin_user.clone(), state.admin_password.clone())
        {
            None => axum::Json(serde_json::json!({
                "configured": false,
                "reason": "set security.oidc_issuer to a Keycloak realm URL and FLOWR_SECURITY_OIDC_ADMIN_PASSWORD",
            })).into_response(),
            Some(cfg) => match a3t_security::oidc_admin::admin_token(&state.client, &cfg).await {
                Ok(_) => axum::Json(serde_json::json!({
                    "configured": true, "connected": true,
                    "base_url": cfg.base_url, "realm": cfg.realm,
                })).into_response(),
                Err(e) => axum::Json(serde_json::json!({
                    "configured": true, "connected": false,
                    "base_url": cfg.base_url, "realm": cfg.realm, "error": e.to_string(),
                })).into_response(),
            },
        }
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = &state;
        (axum::http::StatusCode::NOT_IMPLEMENTED,
            axum::Json(serde_json::json!({ "error": "IdP management is an EE feature" }))).into_response()
    }
}

/// `GET /security/idp/users` — list realm users (projected to id/username/email/roles).
async fn idp_list_users_handler(
    Extension(state): Extension<IdpAdminState>,
    principal: Option<Extension<flowr_policy::Principal>>,
) -> impl IntoResponse {
    if !is_infosec(principal.as_ref().map(|e| &e.0)) {
        return (axum::http::StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "error": "SuperAdmin or SecurityAdmin role required" }))).into_response();
    }
    #[cfg(feature = "ee")]
    {
        use a3t_security::oidc_admin as kc;
        let cfg = match kc::IdpAdminConfig::from_issuer(&state.issuer, state.admin_user.clone(), state.admin_password.clone()) {
            Some(c) => c,
            None => return (axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({ "error": "IdP not configured" }))).into_response(),
        };
        let token = match kc::admin_token(&state.client, &cfg).await {
            Ok(t) => t,
            Err(e) => return (axum::http::StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
        };
        match kc::list_users(&state.client, &cfg, &token).await {
            Ok(users) => {
                let projected: Vec<_> = users.iter().map(|u| serde_json::json!({
                    "id": u.get("id").and_then(|v| v.as_str()),
                    "username": u.get("username").and_then(|v| v.as_str()),
                    "email": u.get("email").and_then(|v| v.as_str()),
                    "enabled": u.get("enabled").and_then(|v| v.as_bool()),
                })).collect();
                axum::Json(serde_json::json!({ "count": projected.len(), "users": projected })).into_response()
            }
            Err(e) => (axum::http::StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
        }
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = &state;
        (axum::http::StatusCode::NOT_IMPLEMENTED,
            axum::Json(serde_json::json!({ "error": "IdP management is an EE feature" }))).into_response()
    }
}

/// `POST /security/idp/users` — create a Keycloak user and assign realm roles.
async fn idp_create_user_handler(
    Extension(state): Extension<IdpAdminState>,
    principal: Option<Extension<flowr_policy::Principal>>,
    axum::Json(body): axum::Json<IdpNewUser>,
) -> impl IntoResponse {
    if !is_infosec(principal.as_ref().map(|e| &e.0)) {
        return (axum::http::StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "error": "SuperAdmin or SecurityAdmin role required" }))).into_response();
    }
    #[cfg(feature = "ee")]
    {
        use a3t_security::oidc_admin as kc;
        if body.username.trim().is_empty() || body.password.is_empty() {
            return (axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({ "error": "username and password are required" }))).into_response();
        }
        if let Err(msg) = check_role_grant(principal.as_ref().map(|e| &e.0), &body.roles) {
            return idp_refuse(axum::http::StatusCode::FORBIDDEN, msg);
        }
        let cfg = match kc::IdpAdminConfig::from_issuer(&state.issuer, state.admin_user.clone(), state.admin_password.clone()) {
            Some(c) => c,
            None => return (axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({ "error": "IdP not configured" }))).into_response(),
        };
        let token = match kc::admin_token(&state.client, &cfg).await {
            Ok(t) => t,
            Err(e) => return (axum::http::StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
        };
        let new_user = kc::NewUser {
            username: body.username.clone(),
            email: body.email.clone(),
            password: body.password.clone(),
            name: body.name.clone(),
            temporary: body.temporary,
            roles: body.roles.clone(),
        };
        match kc::create_user(&state.client, &cfg, &token, &new_user).await {
            Ok(id) => {
                info!(username = %body.username, roles = ?body.roles, "created Keycloak user");
                (axum::http::StatusCode::CREATED,
                    axum::Json(serde_json::json!({ "created": true, "id": id, "username": body.username, "roles": body.roles }))).into_response()
            }
            Err(kc::IdpAdminError::Api { status, body: b }) if status == 409 => {
                let _ = b;
                (axum::http::StatusCode::CONFLICT,
                    axum::Json(serde_json::json!({ "error": format!("user '{}' already exists", body.username) }))).into_response()
            }
            Err(e) => (axum::http::StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
        }
    }
    #[cfg(not(feature = "ee"))]
    {
        let _ = (&state, &body);
        (axum::http::StatusCode::NOT_IMPLEMENTED,
            axum::Json(serde_json::json!({ "error": "IdP management is an EE feature" }))).into_response()
    }
}

/// Body for `PUT /security/idp/users/:id` — all fields optional (partial update).
#[derive(serde::Deserialize)]
struct IdpUpdateUser {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    temporary: bool,
    /// When present, replaces the user's manageable realm roles.
    #[serde(default)]
    roles: Option<Vec<String>>,
}

/// `GET /security/idp/users/:id` — one user's detail + its realm roles (for edit).
async fn idp_get_user_handler(
    Extension(state): Extension<IdpAdminState>,
    principal: Option<Extension<flowr_policy::Principal>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    if !is_infosec(principal.as_ref().map(|e| &e.0)) {
        return (axum::http::StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "error": "SuperAdmin or SecurityAdmin role required" }))).into_response();
    }
    #[cfg(feature = "ee")]
    {
        use a3t_security::oidc_admin as kc;
        let (cfg, token) = match idp_cfg_token(&state).await {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        let user = match kc::get_user(&state.client, &cfg, &token, &id).await {
            Ok(u) => u,
            Err(e) => return (axum::http::StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
        };
        let roles = kc::user_realm_roles(&state.client, &cfg, &token, &id).await.unwrap_or_default();
        let first = user.get("firstName").and_then(|v| v.as_str()).unwrap_or("");
        let last = user.get("lastName").and_then(|v| v.as_str()).unwrap_or("");
        let name = format!("{first} {last}").trim().to_string();
        axum::Json(serde_json::json!({
            "id": user.get("id").and_then(|v| v.as_str()),
            "username": user.get("username").and_then(|v| v.as_str()),
            "email": user.get("email").and_then(|v| v.as_str()),
            "name": name,
            "enabled": user.get("enabled").and_then(|v| v.as_bool()),
            // Manageable roles only — hide Keycloak's default composite.
            "roles": roles.into_iter().filter(|r| !r.starts_with("default-roles")).collect::<Vec<_>>(),
        })).into_response()
    }
    #[cfg(not(feature = "ee"))]
    { let _ = (&state, &id); idp_ee_only() }
}

/// `PUT /security/idp/users/:id` — update profile / password / roles.
async fn idp_update_user_handler(
    Extension(state): Extension<IdpAdminState>,
    principal: Option<Extension<flowr_policy::Principal>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::Json(body): axum::Json<IdpUpdateUser>,
) -> impl IntoResponse {
    if !is_infosec(principal.as_ref().map(|e| &e.0)) {
        return (axum::http::StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "error": "SuperAdmin or SecurityAdmin role required" }))).into_response();
    }
    #[cfg(feature = "ee")]
    {
        use a3t_security::oidc_admin as kc;
        let who = principal.as_ref().map(|e| &e.0);
        if let Some(roles) = &body.roles {
            // An empty list would strip every role; to stop someone signing in, disable them.
            if roles.is_empty() {
                return idp_refuse(axum::http::StatusCode::BAD_REQUEST,
                    "A person needs at least one role. To stop them signing in, disable the account instead.");
            }
            if let Err(msg) = check_role_grant(who, roles) {
                return idp_refuse(axum::http::StatusCode::FORBIDDEN, msg);
            }
        }
        let (cfg, token) = match idp_cfg_token(&state).await { Ok(v) => v, Err(resp) => return resp };
        if let Err(resp) = guard_infosec_target(&state, &cfg, &token, who, &id).await { return resp; }
        let upd = kc::UpdateUser {
            name: body.name.clone(),
            email: body.email.clone(),
            enabled: body.enabled,
            password: body.password.clone(),
            temporary: body.temporary,
            roles: body.roles.clone(),
        };
        match kc::update_user(&state.client, &cfg, &token, &id, &upd).await {
            Ok(()) => { info!(user_id = %id, "updated Keycloak user");
                axum::Json(serde_json::json!({ "updated": true, "id": id })).into_response() }
            Err(e) => (axum::http::StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
        }
    }
    #[cfg(not(feature = "ee"))]
    { let _ = (&state, &id, &body); idp_ee_only() }
}

/// `DELETE /security/idp/users/:id` — delete a user.
async fn idp_delete_user_handler(
    Extension(state): Extension<IdpAdminState>,
    principal: Option<Extension<flowr_policy::Principal>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    if !is_infosec(principal.as_ref().map(|e| &e.0)) {
        return (axum::http::StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "error": "SuperAdmin or SecurityAdmin role required" }))).into_response();
    }
    #[cfg(feature = "ee")]
    {
        use a3t_security::oidc_admin as kc;
        let (cfg, token) = match idp_cfg_token(&state).await { Ok(v) => v, Err(resp) => return resp };
        if let Err(resp) = guard_infosec_target(&state, &cfg, &token, principal.as_ref().map(|e| &e.0), &id).await { return resp; }
        match kc::delete_user(&state.client, &cfg, &token, &id).await {
            Ok(()) => { info!(user_id = %id, "deleted Keycloak user");
                (axum::http::StatusCode::NO_CONTENT, axum::Json(serde_json::json!({ "deleted": true }))).into_response() }
            Err(e) => (axum::http::StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
        }
    }
    #[cfg(not(feature = "ee"))]
    { let _ = (&state, &id); idp_ee_only() }
}

/// Shared: build the IdP config + fetch an admin token, or return an error response.
#[cfg(feature = "ee")]
async fn idp_cfg_token(state: &IdpAdminState)
    -> Result<(a3t_security::oidc_admin::IdpAdminConfig, String), axum::response::Response>
{
    use a3t_security::oidc_admin as kc;
    let cfg = kc::IdpAdminConfig::from_issuer(&state.issuer, state.admin_user.clone(), state.admin_password.clone())
        .ok_or_else(|| (axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "error": "IdP not configured" }))).into_response())?;
    let token = kc::admin_token(&state.client, &cfg).await
        .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({ "error": e.to_string() }))).into_response())?;
    Ok((cfg, token))
}

#[cfg(not(feature = "ee"))]
fn idp_ee_only() -> axum::response::Response {
    (axum::http::StatusCode::NOT_IMPLEMENTED,
        axum::Json(serde_json::json!({ "error": "IdP management is an EE feature" }))).into_response()
}

/// RFC 9728 — OAuth 2.0 Protected Resource Metadata. Tells an MCP client what
/// this resource is and which authorization server issues tokens for it.
async fn oauth_protected_resource() -> impl IntoResponse {
    let r = resource_uri();
    // When an external OIDC provider (Keycloak) is the token authority, point MCP
    // clients at IT — so Claude Desktop runs the browser OAuth (auth-code + PKCE)
    // flow against Keycloak, which has the authorization_endpoint + DCR. In HS256
    // mode we advertise ourselves (password-grant token endpoint).
    let authn = std::env::var("FLOWR_SECURITY_AUTHN").unwrap_or_default();
    let issuer = std::env::var("FLOWR_SECURITY_OIDC_ISSUER").unwrap_or_default();
    let auth_server = if matches!(authn.as_str(), "oidc" | "dual") && !issuer.is_empty() {
        issuer
    } else {
        r.clone()
    };
    axum::Json(serde_json::json!({
        "resource": r,
        "authorization_servers": [auth_server],
        "bearer_methods_supported": ["header"],
        "resource_documentation": r,
    }))
}

/// RFC 8414 — OAuth 2.0 Authorization Server Metadata.
///
/// flowr issues bearer JWTs from a password login rather than a full OAuth
/// authorization-code flow, so the interactive `authorization_endpoint` /
/// PKCE / dynamic-registration machinery is not yet present. This advertises
/// what does exist (the token-issuing login endpoint and the audience-bound
/// bearer scheme); the authorization-code flow is intentionally future work.
async fn oauth_authorization_server() -> impl IntoResponse {
    let r = resource_uri();
    axum::Json(serde_json::json!({
        "issuer": r,
        "token_endpoint": format!("{}/auth/login", r),
        "token_endpoint_auth_methods_supported": ["none"],
        "grant_types_supported": ["password"],
        "response_types_supported": ["token"],
        "code_challenge_methods_supported": ["S256"],
    }))
}
