//! An org's policy files: its roles, rules and access mappings, kept in the
//! org's own GitHub repo (decided 17 Nov 2026). A3T never hosts them in a
//! repo of its own: it keeps a copy in its database and enforces from that.
//!
//! An org connects a repo (`owner/name`, a branch or tag or commit, a folder)
//! with a read token the org gives A3T (a registry secret in one of the org's
//! projects); a public repo needs none. Two ways the copy is refreshed:
//!
//! - `import`: only when someone asks (`POST /security/orgs/{org}/sync`).
//! - `live`: pulled at runtime: on every push (GitHub webhook) and every 60 s.
//!   Without a token GitHub allows 60 calls an hour, so a source with no token
//!   isn't polled: it follows pushes by webhook, or is synced when asked.
//!
//! Either way the database copy is the last good set. A sync fetches the whole
//! folder at one commit, compiles every file and checks it loads with every
//! other policy; any error in any file refuses the whole commit and the last
//! good set stays enforced, so a broken push or an unreachable GitHub never
//! drops an org's rules.
//!
//! Files (below the folder): `policies/*.yaml` (`kind: a3t/policy`),
//! `roles/*.yaml` (`kind: a3t/role`), `access/*.yaml` (`kind: a3t/access`).
//!
//! - `GET|PUT|DELETE /security/orgs/{org}/source`
//! - `POST /security/orgs/{org}/sync`
//! - `GET /security/orgs/{org}/files`: every kept file, and whether it is enforced
//! - `PUT /security/orgs/{org}/files/enforced` `{enforced: [path…]}`: which kept
//!   files are enforced. The others stay kept (and listed) but out of the policy
//!   set; the chosen set must compile on its own, so a rule can't lose the role
//!   it names. A new file from GitHub is enforced; a switched-off one stays off
//!   across commits until switched back on.
//! - `POST /webhook/_a3t/github` (public; each org's webhook secret signs it)
//!
//! Org admins and org security admins manage their own org; platform
//! SuperAdmin and SecurityAdmin manage any.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use flowr_policy::Principal;
use flowr_storage::StorageBackend;

use crate::handlers::call_lancedb::{query_platform_table, sql_quote, DEFAULT_PLATFORM_DB_PATH, PLATFORM_DB_RESOURCE};
use crate::platform_db::{open_platform_db, PlatformDb};

pub const SOURCES: &str = "org_policy_sources";
pub const FILES: &str = "org_policy_files";
/// Kept files switched off: `(org, path)`; everything else kept is enforced.
pub const OFF: &str = "org_policy_off";
const OFF_COLUMNS: [&str; 4] = ["org", "path", "updated_by", "updated_at"];
const SOURCE_COLUMNS: [&str; 12] = [
    "org", "repo", "git_ref", "folder", "token_secret", "webhook_secret", "mode",
    "last_commit", "last_sync", "last_error", "updated_by", "updated_at",
];
const FILE_COLUMNS: [&str; 6] = ["org", "path", "kind", "content", "commit", "updated_at"];

/// The most files, and the largest file, A3T reads from a repo.
const MAX_FILES: usize = 300;
const MAX_FILE_BYTES: u64 = 256 * 1024;
const POLL: Duration = Duration::from_secs(60);

type Reply = Result<Response, Response>;
type St = State<Arc<dyn StorageBackend>>;
type P = Option<Extension<Principal>>;

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}
fn col(row: &Value, k: &str) -> String {
    row.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}
fn who(p: P) -> Principal {
    p.map(|Extension(p)| p).unwrap_or_else(|| Principal::service("anonymous"))
}
fn both(r: Reply) -> Response {
    r.unwrap_or_else(|e| e)
}
fn internal(e: impl ToString) -> Response {
    err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub fn router(storage: Arc<dyn StorageBackend>) -> Router {
    spawn_poller(storage.clone());
    Router::new()
        .route("/security/orgs/:org/source", get(get_source).put(put_source).delete(delete_source))
        .route("/security/orgs/:org/sync", post(sync_now))
        .route("/security/orgs/:org/files", get(list_files))
        .route("/security/orgs/:org/files/enforced", axum::routing::put(set_enforced))
        .route("/webhook/_a3t/github", post(github_webhook))
        .with_state(storage)
}

// ── Tables ───────────────────────────────────────────────────────────────────

async fn db(s: &dyn StorageBackend) -> Result<Arc<dyn PlatformDb>, String> {
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    for (t, cols) in [(SOURCES, &SOURCE_COLUMNS[..]), (FILES, &FILE_COLUMNS[..]), (OFF, &OFF_COLUMNS[..])] {
        let schema: Map<String, Value> = cols.iter().map(|c| (c.to_string(), json!("string"))).collect();
        db.create_table(t, Some(&schema), "create_if_not_exists").await?;
    }
    Ok(db)
}

async fn source_of(s: &dyn StorageBackend, org: &str) -> Option<Value> {
    query_platform_table(s, SOURCES, Some(format!("org = {}", sql_quote(org))), 1).await.ok()?.into_iter().next()
}

async fn files_of(s: &dyn StorageBackend, org: &str) -> Vec<Value> {
    query_platform_table(s, FILES, Some(format!("org = {}", sql_quote(org))), MAX_FILES * 2).await.unwrap_or_default()
}

/// The org's kept files that are switched off.
async fn off_of(s: &dyn StorageBackend, org: &str) -> std::collections::BTreeSet<String> {
    query_platform_table(s, OFF, Some(format!("org = {}", sql_quote(org))), MAX_FILES * 2).await.unwrap_or_default()
        .iter().map(|r| col(r, "path")).collect()
}

/// The org's projects (their namespaces): where its files may apply.
pub async fn org_projects(s: &dyn StorageBackend, org: &str) -> Vec<String> {
    let filter = format!("owner_kind = 'org' AND owner = {}", sql_quote(org));
    let mut out: Vec<String> = query_platform_table(s, "projects", Some(filter), 10_000).await.unwrap_or_default()
        .iter().map(|r| col(r, "namespace")).filter(|n| !n.is_empty()).collect();
    out.sort();
    out.dedup();
    out
}

/// Every org's enforced files, for the policy set: `(org, projects, [(path, text)])`.
pub async fn all_org_files(s: &dyn StorageBackend) -> Vec<(String, Vec<String>, Vec<(String, String)>)> {
    let rows = query_platform_table(s, FILES, None, 100_000).await.unwrap_or_default();
    let off: std::collections::BTreeSet<(String, String)> = query_platform_table(s, OFF, None, 100_000).await.unwrap_or_default()
        .iter().map(|r| (col(r, "org"), col(r, "path"))).collect();
    let mut by_org: std::collections::BTreeMap<String, Vec<(String, String)>> = Default::default();
    for r in rows {
        if off.contains(&(col(&r, "org"), col(&r, "path"))) {
            continue;
        }
        by_org.entry(col(&r, "org")).or_default().push((col(&r, "path"), col(&r, "content")));
    }
    let mut out = Vec::new();
    for (org, files) in by_org {
        if org.is_empty() {
            continue;
        }
        let projects = org_projects(s, &org).await;
        out.push((org, projects, files));
    }
    out
}

// ── Who may manage an org's files ────────────────────────────────────────────

async fn may_manage(s: &dyn StorageBackend, principal: &Principal, org: &str) -> Result<(), Response> {
    let roles = principal.roles();
    if roles.iter().any(|r| r == flowr_policy::roles::SUPER_ADMIN || r == flowr_policy::roles::SECURITY_ADMIN) {
        return Ok(());
    }
    let nope = || err(StatusCode::NOT_FOUND, format!("no org {org:?} whose security you manage"));
    let Some(user) = flowr_policy::namespace::acting_user(principal) else { return Err(nope()) };
    let org_id = query_platform_table(s, "orgs", Some(format!("slug = {}", sql_quote(org))), 1).await
        .ok().and_then(|r| r.into_iter().next()).map(|r| col(&r, "id")).ok_or_else(nope)?;
    let filter = format!("org_id = {} AND user_id = {}", sql_quote(&org_id), sql_quote(user));
    let role = query_platform_table(s, "org_members", Some(filter), 1).await
        .ok().and_then(|r| r.into_iter().next()).map(|r| col(&r, "role")).unwrap_or_default();
    if role == "admin" || role == "securityadmin" { Ok(()) } else { Err(nope()) }
}

// ── The org's source ─────────────────────────────────────────────────────────

fn valid_repo(r: &str) -> bool {
    let ok = |p: &str| !p.is_empty() && p.len() <= 100 && p != "." && p != ".."
        && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    matches!(r.split_once('/'), Some((o, n)) if ok(o) && ok(n))
}
fn valid_ref(r: &str) -> bool {
    !r.is_empty() && r.len() <= 200 && !r.contains("..") && !r.starts_with('/')
        && r.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
}
fn valid_folder(f: &str) -> bool {
    f.len() <= 200 && !f.split('/').any(|p| p == ".." || p == ".")
        && f.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
}

async fn get_source(State(s): St, p: P, Path(org): Path<String>) -> Response {
    both(async {
        may_manage(s.as_ref(), &who(p), &org).await?;
        let src = source_of(s.as_ref(), &org).await;
        Ok(Json(json!({ "org": org, "source": src.map(|r| public_source(&r)) })).into_response())
    }.await)
}

/// The source as shown: secrets by reference only, never their values.
fn public_source(r: &Value) -> Value {
    json!({
        "repo": col(r, "repo"), "ref": col(r, "git_ref"), "folder": col(r, "folder"), "mode": col(r, "mode"),
        "token_secret": col(r, "token_secret"), "webhook_secret": col(r, "webhook_secret"),
        "last_commit": col(r, "last_commit"), "last_sync": col(r, "last_sync"), "last_error": col(r, "last_error"),
        "updated_by": col(r, "updated_by"), "updated_at": col(r, "updated_at"),
    })
}

/// A secret reference (`<project>/<name>`) in one of the org's projects.
async fn check_secret_ref(s: &dyn StorageBackend, org: &str, r: &str) -> Result<(), Response> {
    let bad = |m: String| err(StatusCode::BAD_REQUEST, m);
    let (ns, name) = r.split_once('/').ok_or_else(|| bad(format!("{r:?}: a secret as <project>/<name>")))?;
    if !org_projects(s, org).await.iter().any(|p| p == ns) {
        return Err(bad(format!("{r:?}: the secret must be in one of {org}'s projects")));
    }
    let res = s.resources().get(ns, name).await.map_err(|_| bad(format!("no secret {r:?}")))?;
    if res.type_ref != "secret" {
        return Err(bad(format!("{r:?} is a {} resource, not a secret", res.type_ref)));
    }
    Ok(())
}

async fn secret_value(s: &dyn StorageBackend, r: &str) -> Option<String> {
    let (ns, name) = r.split_once('/')?;
    let res = s.resources().get(ns, name).await.ok()?;
    ["value", "token", "api_key"].iter().find_map(|k| res.config.get(*k).and_then(|v| v.as_str()))
        .map(String::from).filter(|v| !v.is_empty())
}

async fn put_source(State(s): St, p: P, Path(org): Path<String>, Json(b): Json<Value>) -> Response {
    both(async {
        let principal = who(p);
        let s = s.as_ref();
        may_manage(s, &principal, &org).await?;
        let get = |k: &str| b.get(k).and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
        let (repo, git_ref, folder, mode) = (get("repo"), get("ref"), get("folder").trim_matches('/').to_string(), get("mode"));
        let git_ref = if git_ref.is_empty() { "main".to_string() } else { git_ref };
        let mode = if mode.is_empty() { "live".to_string() } else { mode };
        if !valid_repo(&repo) {
            return Err(err(StatusCode::BAD_REQUEST, "repo: the org's GitHub repo as <owner>/<name>"));
        }
        if !valid_ref(&git_ref) {
            return Err(err(StatusCode::BAD_REQUEST, "ref: a branch, tag or commit"));
        }
        if !valid_folder(&folder) {
            return Err(err(StatusCode::BAD_REQUEST, "folder: a path in the repo, without . or .."));
        }
        if mode != "live" && mode != "import" {
            return Err(err(StatusCode::BAD_REQUEST, "mode: live (pulled on every push and every minute) or import (only when asked)"));
        }
        // No token: a public repo.
        let token_secret = get("token_secret");
        if !token_secret.is_empty() {
            check_secret_ref(s, &org, &token_secret).await?;
        }
        let webhook_secret = get("webhook_secret");
        if !webhook_secret.is_empty() {
            check_secret_ref(s, &org, &webhook_secret).await?;
        }
        let prev = source_of(s, &org).await;
        let keep = |k: &str| prev.as_ref().map(|r| col(r, k)).unwrap_or_default();
        let same_place = prev.as_ref().is_some_and(|r| col(r, "repo") == repo && col(r, "git_ref") == git_ref && col(r, "folder") == folder);
        let row = json!({
            "org": org, "repo": repo, "git_ref": git_ref, "folder": folder, "mode": mode,
            "token_secret": token_secret, "webhook_secret": webhook_secret,
            "last_commit": if same_place { keep("last_commit") } else { String::new() },
            "last_sync": keep("last_sync"), "last_error": "",
            "updated_by": principal.id(), "updated_at": chrono::Utc::now().to_rfc3339(),
        });
        db(s).await.map_err(internal)?.upsert(SOURCES, &["org"], &[row.clone()]).await.map_err(internal)?;
        Ok(Json(json!({ "org": org, "source": public_source(&row), "next": "POST /security/orgs/{org}/sync reads the repo now" })).into_response())
    }.await)
}

async fn delete_source(State(s): St, p: P, Path(org): Path<String>) -> Response {
    both(async {
        may_manage(s.as_ref(), &who(p), &org).await?;
        db(s.as_ref()).await.map_err(internal)?.delete(SOURCES, &format!("org = {}", sql_quote(&org))).await.map_err(internal)?;
        // The kept files stay enforced: disconnecting a repo is not removing the rules.
        Ok(Json(json!({ "org": org, "source": null, "files": "kept and still enforced; PUT /security/orgs/{org}/files/enforced chooses which" })).into_response())
    }.await)
}

async fn list_files(State(s): St, p: P, Path(org): Path<String>) -> Response {
    both(async {
        let s = s.as_ref();
        may_manage(s, &who(p), &org).await?;
        let off = off_of(s, &org).await;
        let mut files: Vec<Value> = files_of(s, &org).await.iter().map(|r| json!({
            "path": col(r, "path"), "kind": col(r, "kind"), "content": col(r, "content"), "commit": col(r, "commit"),
            "enforced": !off.contains(&col(r, "path")),
        })).collect();
        files.sort_by_key(|f| col(f, "path"));
        let src = source_of(s, &org).await;
        Ok(Json(json!({
            "org": org, "projects": org_projects(s, &org).await, "files": files,
            "source": src.map(|r| public_source(&r)),
        })).into_response())
    }.await)
}

/// `PUT /security/orgs/{org}/files/enforced` — which kept files are enforced.
async fn set_enforced(State(s): St, p: P, Path(org): Path<String>, Json(b): Json<Value>) -> Response {
    both(async {
        let principal = who(p);
        let s = s.as_ref();
        may_manage(s, &principal, &org).await?;
        let Some(want) = b.get("enforced").and_then(|v| v.as_array()) else {
            return Err(err(StatusCode::BAD_REQUEST, "enforced: the paths of the kept files to enforce"));
        };
        let want: std::collections::BTreeSet<String> = want.iter().filter_map(|v| v.as_str().map(String::from)).collect();
        let kept: Vec<(String, String)> = files_of(s, &org).await.iter().map(|r| (col(r, "path"), col(r, "content"))).collect();
        if let Some(bad) = want.iter().find(|w| !kept.iter().any(|(p, _)| p == *w)) {
            return Err(err(StatusCode::BAD_REQUEST, format!("{bad}: not one of {org}'s kept files")));
        }
        // What will be enforced must compile on its own and load with every other policy.
        let on: Vec<(String, String)> = kept.iter().filter(|(p, _)| want.contains(p)).cloned().collect();
        let gate = crate::policy_gate::get()
            .ok_or_else(|| err(StatusCode::NOT_IMPLEMENTED, "org policy files need FlowR EE"))?;
        if !on.is_empty() {
            if let Err(errors) = gate.compile_org_files(&org, &org_projects(s, &org).await, &on).await {
                return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({
                    "error": "those files don't work on their own; nothing changed", "errors": errors,
                }))).into_response());
            }
        }
        let now = chrono::Utc::now().to_rfc3339();
        let rows: Vec<Value> = kept.iter().filter(|(p, _)| !want.contains(p))
            .map(|(p, _)| json!({ "org": org, "path": p, "updated_by": principal.id(), "updated_at": now })).collect();
        let db = db(s).await.map_err(internal)?;
        let before = off_of(s, &org).await;
        if !rows.is_empty() {
            db.upsert(OFF, &["org", "path"], &rows).await.map_err(internal)?;
        }
        for p in before.iter().filter(|p| want.contains(*p)) {
            db.delete(OFF, &format!("org = {} AND path = {}", sql_quote(&org), sql_quote(p))).await.map_err(internal)?;
        }
        gate.changed().await;
        info!(org, enforced = on.len(), off = rows.len(), by = %principal.id(), "org policy files enforced");
        Ok(Json(json!({ "org": org, "enforced": on.len(), "off": rows.len() })).into_response())
    }.await)
}

// ── GitHub ───────────────────────────────────────────────────────────────────

fn github_api() -> String {
    std::env::var("FLOWR_GITHUB_API").ok().filter(|v| !v.is_empty())
        .unwrap_or_else(|| "https://api.github.com".into()).trim_end_matches('/').to_string()
}

/// A GitHub API GET, with the org's token when it gave one (`""`: a public repo).
async fn gh(client: &reqwest::Client, token: &str, url: &str, raw: bool) -> Result<reqwest::Response, String> {
    let accept = if raw { "application/vnd.github.raw" } else { "application/vnd.github+json" };
    let mut req = client.get(url);
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let r = req.header("Accept", accept)
        .header("User-Agent", "a3t-flowr").header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(Duration::from_secs(20)).send().await
        .map_err(|e| format!("GitHub did not answer: {}", e.without_url()))?;
    match r.status().as_u16() {
        200 => Ok(r),
        401 => Err("GitHub refused the token (401): check the org's token secret".into()),
        403 | 429 if token.is_empty() && r.headers().get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()) == Some("0") =>
            Err("GitHub's limit for calls without a token is used up (60 an hour): give the source a token".into()),
        403 | 404 => Err(format!("GitHub: {} not found or the token can't read it ({})", url.trim_start_matches(&github_api()), r.status().as_u16())),
        c => Err(format!("GitHub answered {c} for {}", url.trim_start_matches(&github_api()))),
    }
}

/// The commit the source's ref points at now.
async fn resolve_commit(client: &reqwest::Client, token: &str, repo: &str, git_ref: &str) -> Result<String, String> {
    let v: Value = gh(client, token, &format!("{}/repos/{repo}/commits/{git_ref}", github_api()), false).await?
        .json().await.map_err(|e| format!("GitHub commit: {e}"))?;
    v.get("sha").and_then(|s| s.as_str()).map(String::from).ok_or_else(|| "GitHub commit: no sha".into())
}

/// Every A3T file in the folder at `commit`: `(path below the folder, text)`.
async fn fetch_files(client: &reqwest::Client, token: &str, repo: &str, folder: &str, commit: &str) -> Result<Vec<(String, String)>, String> {
    let tree: Value = gh(client, token, &format!("{}/repos/{repo}/git/trees/{commit}?recursive=1", github_api()), false).await?
        .json().await.map_err(|e| format!("GitHub tree: {e}"))?;
    if tree.get("truncated").and_then(|t| t.as_bool()) == Some(true) {
        return Err("the repo is too large for GitHub to list in one call: point the source at a smaller repo or folder".into());
    }
    let prefix = if folder.is_empty() { String::new() } else { format!("{folder}/") };
    let mut wanted = Vec::new();
    for e in tree.get("tree").and_then(|t| t.as_array()).cloned().unwrap_or_default() {
        if col(&e, "type") != "blob" {
            continue;
        }
        let Some(rel) = col(&e, "path").strip_prefix(&prefix).map(String::from) else { continue };
        if flowr_policy::policy_file::kind_of_path(&rel).is_none() {
            continue;
        }
        if e.get("size").and_then(|s| s.as_u64()).unwrap_or(0) > MAX_FILE_BYTES {
            return Err(format!("{rel}: larger than {} KB", MAX_FILE_BYTES / 1024));
        }
        wanted.push((rel, col(&e, "sha")));
    }
    if wanted.len() > MAX_FILES {
        return Err(format!("{} policy files; A3T reads at most {MAX_FILES}", wanted.len()));
    }
    let mut out = Vec::new();
    for (rel, sha) in wanted {
        let text = gh(client, token, &format!("{}/repos/{repo}/git/blobs/{sha}", github_api()), true).await?
            .text().await.map_err(|e| format!("{rel}: {e}"))?;
        out.push((rel, text));
    }
    Ok(out)
}

// ── Sync ─────────────────────────────────────────────────────────────────────

/// Read the org's repo at its ref, compile everything, and keep it if it all
/// compiles. Answers `{commit, files, changed}`, or why not (the last good set
/// stays enforced, and the reason is kept on the source as `last_error`).
pub async fn sync(s: &dyn StorageBackend, org: &str, known_commit: Option<String>) -> Result<Value, (StatusCode, Value)> {
    let fail = |code: StatusCode, v: Value| (code, v);
    let src = source_of(s, org).await
        .ok_or_else(|| fail(StatusCode::NOT_FOUND, json!({ "error": format!("{org} has no policy repo connected") })))?;
    let gate = crate::policy_gate::get()
        .ok_or_else(|| fail(StatusCode::NOT_IMPLEMENTED, json!({ "error": "org policy files need FlowR EE" })))?;
    let note = |e: &str| {
        let mut row = src.clone();
        let e = e.to_string();
        async move {
            row["last_error"] = json!(e);
            row["last_sync"] = json!(chrono::Utc::now().to_rfc3339());
            if let Ok(db) = db(s).await {
                let _ = db.upsert(SOURCES, &["org"], &[row]).await;
            }
        }
    };
    let token_ref = col(&src, "token_secret");
    let token = match secret_value(s, &token_ref).await {
        Some(t) => t,
        None if token_ref.is_empty() => String::new(),
        None => {
            let e = format!("the token secret {:?} is missing or empty", col(&src, "token_secret"));
            note(&e).await;
            return Err(fail(StatusCode::BAD_REQUEST, json!({ "error": e })));
        }
    };
    let client = swf_runtime::handlers::call_http::shared_client();
    let (repo, folder) = (col(&src, "repo"), col(&src, "folder"));
    let commit = match known_commit {
        Some(c) => c,
        None => match resolve_commit(&client, &token, &repo, &col(&src, "git_ref")).await {
            Ok(c) => c,
            Err(e) => {
                note(&e).await;
                return Err(fail(StatusCode::BAD_GATEWAY, json!({ "error": e })));
            }
        },
    };
    let files = match fetch_files(&client, &token, &repo, &folder, &commit).await {
        Ok(f) => f,
        Err(e) => {
            note(&e).await;
            return Err(fail(StatusCode::BAD_GATEWAY, json!({ "error": e, "commit": commit })));
        }
    };
    let projects = org_projects(s, org).await;
    let compiled = match gate.compile_org_files(org, &projects, &files).await {
        Ok(c) => c,
        Err(errors) => {
            let short = commit.get(..7).unwrap_or(&commit);
            note(&format!("commit {short} was not applied: {}", errors.join("; "))).await;
            return Err(fail(StatusCode::UNPROCESSABLE_ENTITY, json!({
                "error": format!("commit {short} was not applied; the last good set stays enforced"),
                "commit": commit, "errors": errors,
            })));
        }
    };
    // What will be enforced (switched-off files stay off) must work on its own too.
    let off = off_of(s, org).await;
    let on: Vec<(String, String)> = files.iter().filter(|(p, _)| !off.contains(p)).cloned().collect();
    if on.len() != files.len() && !on.is_empty() {
        if let Err(errors) = gate.compile_org_files(org, &projects, &on).await {
            let short = commit.get(..7).unwrap_or(&commit);
            note(&format!("commit {short} was not applied: with the switched-off files left out, {}", errors.join("; "))).await;
            return Err(fail(StatusCode::UNPROCESSABLE_ENTITY, json!({
                "error": format!("commit {short} was not applied: the enforced files need one that is switched off"),
                "commit": commit, "errors": errors,
            })));
        }
    }
    // Write the new set before removing what it dropped, so no reload ever
    // sees the org with fewer rules than either commit has.
    let db = db(s).await.map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })))?;
    let now = chrono::Utc::now().to_rfc3339();
    let rows: Vec<Value> = files.iter().map(|(path, text)| json!({
        "org": org, "path": path, "kind": flowr_policy::policy_file::kind_of_path(path).unwrap_or_default(),
        "content": text, "commit": commit, "updated_at": now,
    })).collect();
    let before = files_of(s, org).await;
    let changed = before.len() != rows.len()
        || before.iter().any(|r| !files.iter().any(|(p, t)| *p == col(r, "path") && *t == col(r, "content")));
    if !rows.is_empty() {
        db.upsert(FILES, &["org", "path"], &rows).await.map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })))?;
    }
    for r in &before {
        let path = col(r, "path");
        if !files.iter().any(|(p, _)| *p == path) {
            let _ = db.delete(FILES, &format!("org = {} AND path = {}", sql_quote(org), sql_quote(&path))).await;
        }
    }
    // A switched-off file the repo no longer has is gone; so is its switch.
    for p in off.iter().filter(|p| !files.iter().any(|(f, _)| f == *p)) {
        let _ = db.delete(OFF, &format!("org = {} AND path = {}", sql_quote(org), sql_quote(p))).await;
    }
    let mut row = src.clone();
    row["last_commit"] = json!(commit);
    row["last_sync"] = json!(now);
    row["last_error"] = json!("");
    let _ = db.upsert(SOURCES, &["org"], &[row]).await;
    if changed {
        gate.changed().await;
    }
    info!(org, commit = %commit, files = files.len(), changed, "org policy files synced");
    Ok(json!({ "org": org, "commit": commit, "changed": changed, "files": compiled.get("files"), "access": compiled.get("access") }))
}

async fn sync_now(State(s): St, p: P, Path(org): Path<String>) -> Response {
    if let Err(r) = may_manage(s.as_ref(), &who(p), &org).await {
        return r;
    }
    match sync(s.as_ref(), &org, None).await {
        Ok(v) => Json(v).into_response(),
        Err((code, v)) => (code, Json(v)).into_response(),
    }
}

// ── Runtime pull: every minute, and on every push ────────────────────────────

fn spawn_poller(storage: Arc<dyn StorageBackend>) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(POLL).await;
            if crate::policy_gate::get().is_none() {
                continue;
            }
            poll_once(storage.as_ref()).await;
        }
    });
}

async fn poll_once(s: &dyn StorageBackend) {
    let sources = query_platform_table(s, SOURCES, Some("mode = 'live'".into()), 10_000).await.unwrap_or_default();
    let client = swf_runtime::handlers::call_http::shared_client();
    for src in sources {
        let org = col(&src, "org");
        // No token: not polled (GitHub's 60 calls an hour); pushes reach it by webhook.
        let Some(token) = secret_value(s, &col(&src, "token_secret")).await else { continue };
        match resolve_commit(&client, &token, &col(&src, "repo"), &col(&src, "git_ref")).await {
            Ok(c) if c == col(&src, "last_commit") => {}
            Ok(c) => {
                if let Err((_, e)) = sync(s, &org, Some(c)).await {
                    warn!(org, error = %e, "org policy files: the new commit was not applied");
                }
            }
            Err(e) => warn!(org, error = %e, "org policy files: GitHub not reachable; the kept set stays enforced"),
        }
    }
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let pad = |b: u8| k.iter().map(|x| x ^ b).collect::<Vec<u8>>();
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(msg).finalize();
    Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize().into()
}

/// GitHub's `X-Hub-Signature-256: sha256=<hex>` for `body` under `secret`.
fn signed_by(secret: &str, body: &[u8], header: &str) -> bool {
    let Some(hex) = header.strip_prefix("sha256=") else { return false };
    let mac = hmac_sha256(secret.as_bytes(), body);
    let want: String = mac.iter().map(|b| format!("{b:02x}")).collect();
    // Constant time: the comparison doesn't stop at the first wrong digit.
    want.len() == hex.len() && want.bytes().zip(hex.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// A push to a connected repo. Each org whose source is that repo, in `live`
/// mode, with a webhook secret that signs this body, syncs. A signature no
/// org's secret matches is refused.
async fn github_webhook(State(s): St, headers: HeaderMap, body: Bytes) -> Response {
    let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let (event, sig) = (h("x-github-event"), h("x-hub-signature-256"));
    let payload: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let repo = payload.pointer("/repository/full_name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    if repo.is_empty() || sig.is_empty() {
        return err(StatusCode::UNAUTHORIZED, "a signed GitHub event is needed");
    }
    let sources = query_platform_table(s.as_ref(), SOURCES, None, 10_000).await.unwrap_or_default();
    let mut signed = Vec::new();
    for src in sources.iter().filter(|r| col(r, "repo").eq_ignore_ascii_case(&repo)) {
        let Some(secret) = secret_value(s.as_ref(), &col(src, "webhook_secret")).await else { continue };
        if signed_by(&secret, &body, &sig) {
            signed.push(src.clone());
        }
    }
    if signed.is_empty() {
        return err(StatusCode::UNAUTHORIZED, "no connected org's webhook secret signs this event");
    }
    if event == "ping" {
        return Json(json!({ "ok": true, "orgs": signed.iter().map(|r| col(r, "org")).collect::<Vec<_>>() })).into_response();
    }
    if event != "push" {
        return Json(json!({ "ignored": event })).into_response();
    }
    let pushed = payload.get("ref").and_then(|v| v.as_str()).unwrap_or_default();
    let mut synced = Vec::new();
    for src in signed {
        let r = col(&src, "git_ref");
        let follows = pushed == format!("refs/heads/{r}") || pushed == format!("refs/tags/{r}");
        if col(&src, "mode") != "live" || !follows {
            continue;
        }
        let org = col(&src, "org");
        let after = payload.get("after").and_then(|v| v.as_str()).map(String::from)
            .filter(|c| c.len() == 40 && c.chars().all(|x| x.is_ascii_hexdigit()));
        let outcome = match sync(s.as_ref(), &org, after).await {
            Ok(v) => json!({ "org": org, "commit": v.get("commit"), "changed": v.get("changed") }),
            Err((_, e)) => json!({ "org": org, "error": e.get("error"), "errors": e.get("errors") }),
        };
        synced.push(outcome);
    }
    Json(json!({ "synced": synced })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_githubs_documented_example() {
        // docs.github.com "Validating webhook deliveries": secret "It's a Secret to Everybody", body "Hello, World!".
        let sig = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        assert!(signed_by("It's a Secret to Everybody", b"Hello, World!", sig));
        assert!(!signed_by("another secret", b"Hello, World!", sig));
        assert!(!signed_by("It's a Secret to Everybody", b"Hello, World?", sig));
        assert!(!signed_by("It's a Secret to Everybody", b"Hello, World!", "sha1=abc"));
    }

    #[test]
    fn sources_name_only_safe_places() {
        assert!(valid_repo("acme/security-policies"));
        assert!(!valid_repo("acme/../x") && !valid_repo("acme") && !valid_repo("acme/x y") && !valid_repo("../x"));
        assert!(valid_ref("main") && valid_ref("release/v3") && valid_ref("v3.1"));
        assert!(!valid_ref("../main") && !valid_ref("/main") && !valid_ref("a b") && !valid_ref(""));
        assert!(valid_folder("") && valid_folder("a3t") && valid_folder("teams/sec"));
        assert!(!valid_folder("../x") && !valid_folder("a/./b"));
    }
}
