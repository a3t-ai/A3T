//! An org's policy files: its roles, rules and access mappings, kept as files
//! in A3T's database (decided 17 Nov 2026: no GitHub). They are edited in the
//! console and Studio like any other YAML, and every save is kept as a version.
//!
//! Files: `policies/<name>.yaml` (`kind: a3t/policy`), `roles/<name>.yaml`
//! (`kind: a3t/role`), `access/<name>.yaml` (`kind: a3t/access`).
//!
//! - `GET    /security/orgs/{org}/files`: every file, and whether it is enforced
//! - `GET    /security/orgs/{org}/file?path=`: one file and its versions
//! - `PUT    /security/orgs/{org}/file?path=` `{content}`: save it (a new version)
//! - `DELETE /security/orgs/{org}/file?path=`: remove it (its versions are kept)
//! - `POST   /security/orgs/{org}/validate` `{files?: [{path, content}], removed?: [path]}`:
//!   would the org's enforced files, with these drafts, work? Nothing is saved.
//! - `PUT    /security/orgs/{org}/files/enforced` `{enforced: [path…]}`: which
//!   files are enforced; the others stay kept and listed but out of the policy set.
//!
//! A change is saved only if the files that would then be enforced compile
//! together and load with every other policy (a rule can't lose the role it
//! names); otherwise nothing changes and the errors say why.
//!
//! Org admins and org security admins manage their own org; platform
//! SuperAdmin and SecurityAdmin manage any.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use serde_json::{json, Map, Value};
use tracing::info;

use flowr_policy::Principal;
use flowr_storage::StorageBackend;

use crate::handlers::call_lancedb::{query_platform_table, sql_quote, DEFAULT_PLATFORM_DB_PATH, PLATFORM_DB_RESOURCE};
use crate::platform_db::{open_platform_db, PlatformDb};

/// The org's files as they are now: `commit` holds the file's version (`v<n>`).
pub const FILES: &str = "org_policy_files";
const FILE_COLUMNS: [&str; 6] = ["org", "path", "kind", "content", "commit", "updated_at"];
/// Files switched off: `(org, path)`; every other file is enforced.
pub const OFF: &str = "org_policy_off";
const OFF_COLUMNS: [&str; 4] = ["org", "path", "updated_by", "updated_at"];
/// Files in shadow mode (GOV-2): `(org, path)`, also in `OFF` — evaluated and
/// logged (`policy:shadow`), never enforced.
pub const SHADOW: &str = "org_policy_shadow";
const SHADOW_COLUMNS: [&str; 4] = ["org", "path", "updated_by", "updated_at"];
/// Every version of every file, and every removal and switch, with who and when.
pub const HISTORY: &str = "org_policy_history";
const HISTORY_COLUMNS: [&str; 8] = ["org", "path", "version", "action", "kind", "content", "by", "at"];

/// The most files an org keeps, and the largest file.
const MAX_FILES: usize = 300;
const MAX_FILE_BYTES: usize = 256 * 1024;

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
    Router::new()
        .route("/security/orgs/:org/files", get(list_files))
        .route("/security/orgs/:org/files/enforced", put(set_enforced))
        .route("/security/orgs/:org/files/shadow", put(set_shadow))
        .route("/security/orgs/:org/file", get(get_file).put(put_file).delete(delete_file))
        .route("/security/orgs/:org/validate", post(validate))
        .merge(crate::policy_changes::routes())
        .merge(crate::audit_retention::routes())
        .merge(crate::access_reviews::routes())
        .with_state(storage)
}

// ── Tables ───────────────────────────────────────────────────────────────────

async fn db(s: &dyn StorageBackend) -> Result<Arc<dyn PlatformDb>, String> {
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    for (t, cols) in [(FILES, &FILE_COLUMNS[..]), (OFF, &OFF_COLUMNS[..]), (SHADOW, &SHADOW_COLUMNS[..]), (HISTORY, &HISTORY_COLUMNS[..])] {
        let schema: Map<String, Value> = cols.iter().map(|c| (c.to_string(), json!("string"))).collect();
        db.create_table(t, Some(&schema), "create_if_not_exists").await?;
    }
    Ok(db)
}

async fn files_of(s: &dyn StorageBackend, org: &str) -> Vec<Value> {
    query_platform_table(s, FILES, Some(format!("org = {}", sql_quote(org))), MAX_FILES * 2).await.unwrap_or_default()
}

/// The org's enforced files, `(path, text)` (RT-1 reads its egress files).
pub async fn enforced_files(s: &dyn StorageBackend, org: &str) -> Vec<(String, String)> {
    let off = off_of(s, org).await;
    files_of(s, org).await.iter().map(|r| (col(r, "path"), col(r, "content"))).filter(|(p, _)| !off.contains(p)).collect()
}

/// The org's files that are switched off.
async fn off_of(s: &dyn StorageBackend, org: &str) -> BTreeSet<String> {
    query_platform_table(s, OFF, Some(format!("org = {}", sql_quote(org))), MAX_FILES * 2).await.unwrap_or_default()
        .iter().map(|r| col(r, "path")).collect()
}

/// The org's files in shadow mode.
async fn shadow_of(s: &dyn StorageBackend, org: &str) -> BTreeSet<String> {
    query_platform_table(s, SHADOW, Some(format!("org = {}", sql_quote(org))), MAX_FILES * 2).await.unwrap_or_default()
        .iter().map(|r| col(r, "path")).collect()
}

/// GOV-5, once: an org whose sign-in settings still hold a group → role map
/// gets it as its policy file `access/org.yaml` (enforced, version v1, noted
/// as moved), and the map leaves the settings. Idempotent; run at start.
pub async fn move_sso_maps(s: &dyn StorageBackend) {
    let Ok(rows) = query_platform_table(s, "orgs", None, 10_000).await else { return };
    for r in rows {
        let mut settings: Value = serde_json::from_str(&col(&r, "settings")).unwrap_or(Value::Null);
        let Some(sso) = settings.get_mut("sso").and_then(|v| v.as_object_mut()) else { continue };
        if !sso.contains_key("role_map") && !sso.contains_key("default_role") {
            continue;
        }
        let org = col(&r, "slug");
        let map: std::collections::BTreeMap<String, String> = sso.get("role_map").and_then(|m| m.as_object())
            .map(|m| m.iter().filter_map(|(g, v)| v.as_str().map(|v| (g.clone(), v.to_string()))).collect()).unwrap_or_default();
        let default = sso.get("default_role").and_then(|v| v.as_str()).unwrap_or("member").to_string();
        let have = files_of(s, &org).await.iter().any(|f| col(f, "path") == "access/org.yaml");
        let Ok(db) = db(s).await else { return };
        if !have {
            let mut y = format!("kind: a3t/org-access\ndescription: Which of {org}'s sign-in groups get which org role (moved from its sign-in settings).\n");
            y.push_str("groups:\n");
            if map.is_empty() { y.push_str("  {}\n"); }
            for (g, role) in &map { y.push_str(&format!("  {}: {}\n", serde_json::to_string(g).unwrap_or_default(), role)); }
            if !default.is_empty() { y.push_str(&format!("default: {default}\n")); }
            let y = y.replace("groups:\n  {}\n", "groups: {}\n");
            if flowr_policy::policy_file::OrgAccessFile::parse(&y).is_err() {
                tracing::warn!(org, "sign-in role map not moved: it doesn't make a valid access/org.yaml");
                continue;
            }
            let now = chrono::Utc::now().to_rfc3339();
            if db.upsert(FILES, &["org", "path"], &[json!({ "org": org, "path": "access/org.yaml", "kind": "org-access", "content": y, "commit": "v1", "updated_at": now })]).await.is_err() {
                continue;
            }
            let _ = note(db.as_ref(), &org, "access/org.yaml", "v1", "create", "org-access", &y, "moved from sign-in settings").await;
        }
        sso.remove("role_map");
        sso.remove("default_role");
        let mut values = Map::new();
        values.insert("settings".into(), json!(settings.to_string()));
        let _ = db.update("orgs", &format!("id = {}", sql_quote(&col(&r, "id"))), &values).await;
        tracing::info!(org, "sign-in role map moved to access/org.yaml");
    }
}

/// An org's id, from its slug (the decision log is kept per org id).
pub async fn org_id(s: &dyn StorageBackend, org: &str) -> Option<String> {
    query_platform_table(s, "orgs", Some(format!("slug = {}", sql_quote(org))), 1).await
        .ok().and_then(|r| r.into_iter().next()).map(|r| col(&r, "id")).filter(|i| !i.is_empty())
}

/// What the org's enforced files would be after a change (GOV-2 replay): the
/// enforced files with `drafts` in place, `removed` taken out and `switch_on`
/// switched on (a shadow file being promoted). `(projects, [(path, text)])`.
pub async fn candidate(s: &dyn StorageBackend, org: &str, drafts: &[(String, String)], removed: &[String], switch_on: &[String]) -> (Vec<String>, Vec<(String, String)>) {
    let files = files_of(s, org).await;
    let mut off = off_of(s, org).await;
    for p in switch_on {
        off.remove(p);
    }
    (org_projects(s, org).await, enforced_with(&files, &off, drafts, removed))
}

/// Every org's shadow files: `(org, [(path, text)])` (GOV-2).
pub async fn all_shadow_files(s: &dyn StorageBackend) -> std::collections::BTreeMap<String, Vec<(String, String)>> {
    let shadow: BTreeSet<(String, String)> = query_platform_table(s, SHADOW, None, 100_000).await.unwrap_or_default()
        .iter().map(|r| (col(r, "org"), col(r, "path"))).collect();
    let mut out: std::collections::BTreeMap<String, Vec<(String, String)>> = Default::default();
    if shadow.is_empty() {
        return out;
    }
    for r in query_platform_table(s, FILES, None, 100_000).await.unwrap_or_default() {
        if shadow.contains(&(col(&r, "org"), col(&r, "path"))) {
            out.entry(col(&r, "org")).or_default().push((col(&r, "path"), col(&r, "content")));
        }
    }
    for v in out.values_mut() {
        v.sort();
    }
    out
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
    let off: BTreeSet<(String, String)> = query_platform_table(s, OFF, None, 100_000).await.unwrap_or_default()
        .iter().map(|r| (col(r, "org"), col(r, "path"))).collect();
    let mut by_org: std::collections::BTreeMap<String, Vec<(String, String)>> = Default::default();
    for r in rows {
        if off.contains(&(col(&r, "org"), col(&r, "path"))) {
            continue;
        }
        by_org.entry(col(&r, "org")).or_default().push((col(&r, "path"), col(&r, "content")));
    }
    let mut out = Vec::new();
    for (org, mut files) in by_org {
        if org.is_empty() {
            continue;
        }
        // In path order: the same set builds the same way every time.
        files.sort();
        let projects = org_projects(s, &org).await;
        out.push((org, projects, files));
    }
    out
}

/// Would these files, enforced together for `org`, work? Nothing enforced works.
async fn check(s: &dyn StorageBackend, org: &str, set: &[(String, String)]) -> Result<Value, Vec<String>> {
    if set.is_empty() {
        return Ok(json!({ "files": [] }));
    }
    let gate = crate::policy_gate::get().ok_or_else(|| vec!["org policy files need FlowR EE".to_string()])?;
    gate.compile_org_files(org, &org_projects(s, org).await, set).await
}

/// The org's enforced files, with `drafts` in place (or added) and `removed` left out.
/// A draft of a switched-off file is checked as if it were on.
fn enforced_with(files: &[Value], off: &BTreeSet<String>, drafts: &[(String, String)], removed: &[String]) -> Vec<(String, String)> {
    let mut set: Vec<(String, String)> = files.iter()
        .map(|r| (col(r, "path"), col(r, "content")))
        .filter(|(p, _)| !off.contains(p) && !removed.contains(p) && !drafts.iter().any(|(d, _)| d == p))
        .collect();
    set.extend(drafts.iter().cloned());
    set.sort();
    set
}

async fn note(db: &dyn PlatformDb, org: &str, path: &str, version: &str, action: &str, kind: &str, content: &str, by: &str) -> Result<(), String> {
    db.insert(HISTORY, &[json!({
        "org": org, "path": path, "version": version, "action": action, "kind": kind,
        "content": content, "by": by, "at": chrono::Utc::now().to_rfc3339(),
    })]).await.map(|_| ())
}

async fn changed() {
    if let Some(gate) = crate::policy_gate::get() {
        gate.changed().await;
    }
}

// ── Who may manage an org's files ────────────────────────────────────────────

async fn org_exists(s: &dyn StorageBackend, org: &str) -> bool {
    query_platform_table(s, "orgs", Some(format!("slug = {}", sql_quote(org))), 1).await
        .map(|r| !r.is_empty()).unwrap_or(false)
}

fn platform_infosec(principal: &Principal) -> bool {
    principal.roles().iter().any(|r| r == flowr_policy::roles::SUPER_ADMIN || r == flowr_policy::roles::SECURITY_ADMIN)
}

/// Does `principal` manage the security of the org with this id: platform
/// InfoSec, or the org's admins and security admins?
pub async fn manages_org_security(s: &dyn StorageBackend, principal: &Principal, org_id: &str) -> bool {
    if platform_infosec(principal) {
        return true;
    }
    let Some(user) = flowr_policy::namespace::acting_user(principal) else { return false };
    if org_id.is_empty() {
        return false;
    }
    let filter = format!("org_id = {} AND user_id = {}", sql_quote(org_id), sql_quote(user));
    let role = query_platform_table(s, "org_members", Some(filter), 1).await
        .ok().and_then(|r| r.into_iter().next()).map(|r| col(&r, "role")).unwrap_or_default();
    role == "admin" || role == "securityadmin"
}

pub(crate) async fn may_manage(s: &dyn StorageBackend, principal: &Principal, org: &str) -> Result<(), Response> {
    if platform_infosec(principal) {
        return Ok(());
    }
    let nope = || err(StatusCode::NOT_FOUND, format!("no org {org:?} whose security you manage"));
    let org_id = query_platform_table(s, "orgs", Some(format!("slug = {}", sql_quote(org))), 1).await
        .ok().and_then(|r| r.into_iter().next()).map(|r| col(&r, "id")).ok_or_else(nope)?;
    if manages_org_security(s, principal, &org_id).await { Ok(()) } else { Err(nope()) }
}

/// Both: the org exists, and the caller manages its security.
pub async fn allowed(s: &dyn StorageBackend, principal: &Principal, org: &str) -> Result<(), Response> {
    may_manage(s, principal, org).await?;
    if !org_exists(s, org).await {
        return Err(err(StatusCode::NOT_FOUND, format!("no org {org:?}")));
    }
    Ok(())
}

/// A file's place: `policies|roles|access/<name>.yaml`, a plain name.
fn valid_path(p: &str) -> Result<&'static str, Response> {
    let kind = flowr_policy::policy_file::kind_of_path(p);
    let name = p.split_once('/').map(|(_, f)| f.trim_end_matches(".yaml").trim_end_matches(".yml")).unwrap_or_default();
    let plain = !name.is_empty() && name.len() <= 80 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    match kind {
        Some(k) if plain => Ok(k),
        _ => Err(err(StatusCode::BAD_REQUEST, format!("{p:?}: a file is policies/<name>.yaml, roles/<name>.yaml, access/<name>.yaml, egress/<name>.yaml or networks/<name>.yaml (letters, digits, - and _)"))),
    }
}

#[derive(serde::Deserialize)]
struct PathQ {
    path: String,
}

fn refused(errors: Vec<String>, what: &str) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": format!("{what}; nothing changed"), "errors": errors }))).into_response()
}

// ── Files ────────────────────────────────────────────────────────────────────

async fn list_files(State(s): St, p: P, Path(org): Path<String>) -> Response {
    both(async {
        let s = s.as_ref();
        allowed(s, &who(p), &org).await?;
        let off = off_of(s, &org).await;
        let shadow = shadow_of(s, &org).await;
        let mut files: Vec<Value> = files_of(s, &org).await.iter().map(|r| json!({
            "path": col(r, "path"), "kind": col(r, "kind"), "content": col(r, "content"),
            "version": col(r, "commit"), "updated_at": col(r, "updated_at"),
            "enforced": !off.contains(&col(r, "path")),
            "shadow": shadow.contains(&col(r, "path")),
        })).collect();
        files.sort_by_key(|f| col(f, "path"));
        Ok(Json(json!({ "org": org, "projects": org_projects(s, &org).await, "files": files })).into_response())
    }.await)
}

async fn get_file(State(s): St, p: P, Path(org): Path<String>, Query(q): Query<PathQ>) -> Response {
    both(async {
        let s = s.as_ref();
        allowed(s, &who(p), &org).await?;
        valid_path(&q.path)?;
        let filter = format!("org = {} AND path = {}", sql_quote(&org), sql_quote(&q.path));
        let now = query_platform_table(s, FILES, Some(filter.clone()), 1).await.unwrap_or_default().into_iter().next();
        let mut history = query_platform_table(s, HISTORY, Some(filter), 10_000).await.unwrap_or_default();
        history.sort_by_key(|h| std::cmp::Reverse(col(h, "at")));
        if now.is_none() && history.is_empty() {
            return Err(err(StatusCode::NOT_FOUND, format!("{org} has no file {}", q.path)));
        }
        let off = off_of(s, &org).await;
        Ok(Json(json!({
            "org": org, "path": q.path,
            "file": now.map(|r| json!({ "kind": col(&r, "kind"), "content": col(&r, "content"), "version": col(&r, "commit"),
                "updated_at": col(&r, "updated_at"), "enforced": !off.contains(&q.path) })),
            "history": history.iter().map(|h| json!({
                "version": col(h, "version"), "action": col(h, "action"), "content": col(h, "content"),
                "by": col(h, "by"), "at": col(h, "at"),
            })).collect::<Vec<_>>(),
        })).into_response())
    }.await)
}

async fn put_file(State(s): St, p: P, Path(org): Path<String>, Query(q): Query<PathQ>, Json(b): Json<Value>) -> Response {
    let principal = who(p);
    let Some(content) = b.get("content").and_then(|v| v.as_str()) else {
        return err(StatusCode::BAD_REQUEST, "content: the file's YAML");
    };
    let shadow = b.get("shadow").and_then(|v| v.as_bool()).unwrap_or(false);
    both(save_file(s.as_ref(), &principal, &org, &q.path, content, shadow, None).await)
}

/// Save a file. `approved`: the author of a change a second admin approved
/// (GOV-3); without it, an org with a second approver (or one that lets only
/// its security admins change policy, for an admin) holds the change.
/// `shadow`: a new (or switched-off) rule file goes straight into shadow
/// mode — never enforced on the way.
pub(crate) async fn save_file(s: &dyn StorageBackend, principal: &Principal, org: &str, path: &str, content: &str, shadow: bool, approved: Option<&str>) -> Reply {
    let (org, principal) = (org.to_string(), principal.clone());
    let q = PathQ { path: path.to_string() };
    async {
        allowed(s, &principal, &org).await?;
        let kind = valid_path(&q.path)?;
        if content.len() > MAX_FILE_BYTES {
            return Err(err(StatusCode::BAD_REQUEST, format!("the file is {} bytes; at most {MAX_FILE_BYTES}", content.len())));
        }
        let files = files_of(s, &org).await;
        let prev = files.iter().find(|r| col(r, "path") == q.path);
        if prev.is_none() && files.len() >= MAX_FILES {
            return Err(err(StatusCode::BAD_REQUEST, format!("{org} already has {MAX_FILES} files")));
        }
        if prev.is_some_and(|r| col(r, "content") == content) {
            return Ok(Json(json!({ "org": org, "path": q.path, "version": col(prev.unwrap(), "commit"), "changed": false })).into_response());
        }
        // It must work with the org's enforced files, as if it were on.
        let off = off_of(s, &org).await;
        let set = enforced_with(&files, &off, &[(q.path.clone(), content.to_string())], &[]);
        let compiled = check(s, &org, &set).await.map_err(|e| refused(e, "the file doesn't work with the org's other files"))?;
        if shadow && kind != "policy" {
            return Err(err(StatusCode::BAD_REQUEST, "only rule files (policies/) run in shadow mode"));
        }
        // Shadow mode for a file that is enforced now means switching it off: that
        // is `files/enforced`, not a save.
        if shadow && prev.is_some() && !off.contains(&q.path) {
            return Err(err(StatusCode::CONFLICT, format!("{} is enforced: save it as it is, or switch it off first", q.path)));
        }
        let identity = kind == "org-access";
        let held = approved.is_none() && (crate::policy_changes::four_eyes(s, &org).await || crate::policy_changes::held_by_lock(s, &principal, &org, identity).await);
        if held {
            let summary = format!("{} {}{}", if prev.is_some() { "save" } else { "create" }, q.path, if shadow { " (shadow mode)" } else { "" });
            return crate::policy_changes::propose(s, &principal, &org, "file", "save", &q.path, json!({ "content": content, "shadow": shadow }), &summary).await;
        }
        // Numbering continues across a removal: a path's versions are never reused.
        let num = |v: &str| v.strip_prefix('v').and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
        let path_filter = format!("org = {} AND path = {}", sql_quote(&org), sql_quote(&q.path));
        let last = query_platform_table(s, HISTORY, Some(path_filter), 10_000).await.unwrap_or_default()
            .iter().map(|h| num(&col(h, "version"))).chain(prev.map(|r| num(&col(r, "commit")))).max().unwrap_or(0);
        let n = last + 1;
        let version = format!("v{n}");
        let by = approved.map(str::to_string).unwrap_or_else(|| principal.id().to_string());
        let db = db(s).await.map_err(internal)?;
        // A shadow save is off and in shadow before the file exists, so it is
        // never enforced, not even for a moment.
        let mut off = off;
        if shadow {
            let row = json!({ "org": org, "path": q.path, "updated_by": by, "updated_at": chrono::Utc::now().to_rfc3339() });
            db.upsert(OFF, &["org", "path"], &[row.clone()]).await.map_err(internal)?;
            db.upsert(SHADOW, &["org", "path"], &[row]).await.map_err(internal)?;
            off.insert(q.path.clone());
        }
        db.upsert(FILES, &["org", "path"], &[json!({
            "org": org, "path": q.path, "kind": kind, "content": content, "commit": version, "updated_at": chrono::Utc::now().to_rfc3339(),
        })]).await.map_err(internal)?;
        note(db.as_ref(), &org, &q.path, &version, if prev.is_some() { "save" } else { "create" }, kind, content, &by).await.map_err(internal)?;
        flowr_policy::audit::event(&principal, "policy:write", "PolicyFile", &format!("{org}/{}", q.path), None, Some(&org), None,
            Some(&format!("{} {version}", if prev.is_some() { "saved" } else { "created" })));
        if !off.contains(&q.path) || shadow {
            changed().await;
        }
        info!(org, path = %q.path, %version, %by, shadow, "org policy file saved");
        Ok(Json(json!({ "org": org, "path": q.path, "version": version, "changed": true, "enforced": !off.contains(&q.path),
            "shadow": shadow || shadow_of(s, &org).await.contains(&q.path), "compiled": compiled })).into_response())
    }.await
}

async fn delete_file(State(s): St, p: P, Path(org): Path<String>, Query(q): Query<PathQ>) -> Response {
    let principal = who(p);
    both(remove_file(s.as_ref(), &principal, &org, &q.path, None).await)
}

/// Remove a file (see [`save_file`] for `approved`).
pub(crate) async fn remove_file(s: &dyn StorageBackend, principal: &Principal, org: &str, path: &str, approved: Option<&str>) -> Reply {
    let (org, principal) = (org.to_string(), principal.clone());
    let q = PathQ { path: path.to_string() };
    async {
        allowed(s, &principal, &org).await?;
        let kind = valid_path(&q.path)?;
        let files = files_of(s, &org).await;
        let Some(prev) = files.iter().find(|r| col(r, "path") == q.path) else {
            return Err(err(StatusCode::NOT_FOUND, format!("{org} has no file {}", q.path)));
        };
        let off = off_of(s, &org).await;
        let set = enforced_with(&files, &off, &[], &[q.path.clone()]);
        check(s, &org, &set).await.map_err(|e| refused(e, "the other files need it"))?;
        if approved.is_none() && (crate::policy_changes::four_eyes(s, &org).await || crate::policy_changes::held_by_lock(s, &principal, &org, kind == "org-access").await) {
            return crate::policy_changes::propose(s, &principal, &org, "file", "delete", &q.path, json!({}), &format!("remove {}", q.path)).await;
        }
        let by = approved.map(str::to_string).unwrap_or_else(|| principal.id().to_string());
        let db = db(s).await.map_err(internal)?;
        let filter = format!("org = {} AND path = {}", sql_quote(&org), sql_quote(&q.path));
        db.delete(FILES, &filter).await.map_err(internal)?;
        db.delete(OFF, &filter).await.map_err(internal)?;
        db.delete(SHADOW, &filter).await.map_err(internal)?;
        note(db.as_ref(), &org, &q.path, &col(prev, "commit"), "delete", kind, &col(prev, "content"), &by).await.map_err(internal)?;
        flowr_policy::audit::event(&principal, "policy:delete", "PolicyFile", &format!("{org}/{}", q.path), None, Some(&org), None,
            Some(&format!("deleted {}", col(prev, "commit"))));
        if !off.contains(&q.path) {
            changed().await;
        }
        Ok(Json(json!({ "org": org, "path": q.path, "deleted": true })).into_response())
    }.await
}

/// `POST /security/orgs/{org}/validate` — check drafts; nothing is saved.
async fn validate(State(s): St, p: P, Path(org): Path<String>, Json(b): Json<Value>) -> Response {
    both(async {
        let s = s.as_ref();
        allowed(s, &who(p), &org).await?;
        let mut drafts = Vec::new();
        for f in b.get("files").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
            let path = col(&f, "path");
            valid_path(&path)?;
            drafts.push((path, col(&f, "content")));
        }
        let removed: Vec<String> = b.get("removed").and_then(|v| v.as_array()).cloned().unwrap_or_default()
            .iter().filter_map(|v| v.as_str().map(String::from)).collect();
        let set = enforced_with(&files_of(s, &org).await, &off_of(s, &org).await, &drafts, &removed);
        let checked: Vec<&String> = set.iter().map(|(p, _)| p).collect();
        Ok(Json(match check(s, &org, &set).await {
            Ok(c) => json!({ "ok": true, "checked": checked, "compiled": c }),
            Err(errors) => json!({ "ok": false, "checked": checked, "errors": errors }),
        }).into_response())
    }.await)
}

/// `PUT /security/orgs/{org}/files/enforced` — which files are enforced.
async fn set_enforced(State(s): St, p: P, Path(org): Path<String>, Json(b): Json<Value>) -> Response {
    let principal = who(p);
    both(enforce(s.as_ref(), &principal, &org, &b, None).await)
}

/// Which files are enforced (see [`save_file`] for `approved`).
pub(crate) async fn enforce(s: &dyn StorageBackend, principal: &Principal, org: &str, b: &Value, approved: Option<&str>) -> Reply {
    let (org, principal) = (org.to_string(), principal.clone());
    async {
        allowed(s, &principal, &org).await?;
        let Some(want) = b.get("enforced").and_then(|v| v.as_array()) else {
            return Err(err(StatusCode::BAD_REQUEST, "enforced: the paths of the files to enforce"));
        };
        let want: BTreeSet<String> = want.iter().filter_map(|v| v.as_str().map(String::from)).collect();
        let files = files_of(s, &org).await;
        if let Some(bad) = want.iter().find(|w| !files.iter().any(|r| col(r, "path") == **w)) {
            return Err(err(StatusCode::BAD_REQUEST, format!("{bad}: not one of {org}'s files")));
        }
        // What will be enforced must work on its own and load with every other policy.
        let on: Vec<(String, String)> = files.iter().map(|r| (col(r, "path"), col(r, "content")))
            .filter(|(p, _)| want.contains(p)).collect();
        check(s, &org, &on).await.map_err(|e| refused(e, "those files don't work on their own"))?;
        let before = off_of(s, &org).await;
        let switching: Vec<String> = files.iter().map(|r| col(r, "path")).filter(|p| before.contains(p) == want.contains(p)).collect();
        if switching.is_empty() {
            return Ok(Json(json!({ "org": org, "enforced": on.len(), "off": files.len() - on.len(), "switched": 0 })).into_response());
        }
        let identity = switching.iter().all(|p| flowr_policy::policy_file::kind_of_path(p) == Some("org-access"));
        if approved.is_none() && (crate::policy_changes::four_eyes(s, &org).await || crate::policy_changes::held_by_lock(s, &principal, &org, identity).await) {
            return crate::policy_changes::propose(s, &principal, &org, "file", "enforce", "", json!({ "enforced": want }),
                &format!("switch {}", switching.join(", "))).await;
        }
        let now = chrono::Utc::now().to_rfc3339();
        let by = approved.map(str::to_string).unwrap_or_else(|| principal.id().to_string());
        let db = db(s).await.map_err(internal)?;
        let mut switched = 0;
        for r in &files {
            let path = col(r, "path");
            let was_on = !before.contains(&path);
            let is_on = want.contains(&path);
            if was_on == is_on {
                continue;
            }
            let filter = format!("org = {} AND path = {}", sql_quote(&org), sql_quote(&path));
            if is_on {
                db.delete(OFF, &filter).await.map_err(internal)?;
                db.delete(SHADOW, &filter).await.map_err(internal)?;
            } else {
                db.upsert(OFF, &["org", "path"], &[json!({ "org": org, "path": path, "updated_by": by, "updated_at": now })]).await.map_err(internal)?;
            }
            note(db.as_ref(), &org, &path, &col(r, "commit"), if is_on { "on" } else { "off" }, &col(r, "kind"), &col(r, "content"), &by).await.map_err(internal)?;
            flowr_policy::audit::event(&principal, "policy:switch", "PolicyFile", &format!("{org}/{path}"), None, Some(&org), None,
                Some(if is_on { "switched on" } else { "switched off" }));
            switched += 1;
        }
        if switched > 0 {
            changed().await;
        }
        info!(org, enforced = on.len(), switched, %by, "org policy files enforced");
        Ok(Json(json!({ "org": org, "enforced": on.len(), "off": files.len() - on.len(), "switched": switched })).into_response())
    }.await
}

/// `PUT /security/orgs/{org}/files/shadow` `{path, shadow}` — run a switched-off
/// rule file in shadow mode (GOV-2): evaluated on every call and its would-be
/// refusals logged as `policy:shadow`, never enforced. Switching it on is
/// `files/enforced` (and needs the second approver where the org has one).
async fn set_shadow(State(s): St, p: P, Path(org): Path<String>, Json(b): Json<Value>) -> Response {
    both(async {
        let principal = who(p);
        let s = s.as_ref();
        allowed(s, &principal, &org).await?;
        let path = col(&b, "path");
        let kind = valid_path(&path)?;
        let Some(on) = b.get("shadow").and_then(|v| v.as_bool()) else {
            return Err(err(StatusCode::BAD_REQUEST, "shadow: true or false"));
        };
        if kind != "policy" {
            return Err(err(StatusCode::BAD_REQUEST, "only rule files (policies/) run in shadow mode"));
        }
        let files = files_of(s, &org).await;
        let Some(row) = files.iter().find(|r| col(r, "path") == path) else {
            return Err(err(StatusCode::NOT_FOUND, format!("{org} has no file {path}")));
        };
        if on && !off_of(s, &org).await.contains(&path) {
            return Err(err(StatusCode::CONFLICT, format!("{path} is enforced: switch it off first to run it in shadow mode")));
        }
        if on {
            // It must compile with the enforced files, as it will be evaluated with them.
            let set = enforced_with(&files, &off_of(s, &org).await, &[(path.clone(), col(row, "content"))], &[]);
            check(s, &org, &set).await.map_err(|e| refused(e, "it doesn't work with the org's enforced files"))?;
        }
        let db = db(s).await.map_err(internal)?;
        let filter = format!("org = {} AND path = {}", sql_quote(&org), sql_quote(&path));
        if on {
            db.upsert(SHADOW, &["org", "path"], &[json!({ "org": org, "path": path, "updated_by": principal.id(), "updated_at": chrono::Utc::now().to_rfc3339() })]).await.map_err(internal)?;
        } else {
            db.delete(SHADOW, &filter).await.map_err(internal)?;
        }
        note(db.as_ref(), &org, &path, &col(row, "commit"), if on { "shadow" } else { "unshadow" }, kind, &col(row, "content"), principal.id()).await.map_err(internal)?;
        flowr_policy::audit::event(&principal, "policy:switch", "PolicyFile", &format!("{org}/{path}"), None, Some(&org), None,
            Some(if on { "shadow mode on" } else { "shadow mode off" }));
        changed().await;
        Ok(Json(json!({ "org": org, "path": path, "shadow": on })).into_response())
    }.await)
}
