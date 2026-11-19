//! Files: browse a storage resource (the Files app) — list one level of a
//! bucket or directory, and preview the start of a file.
//!
//! `GET /storage/:ns/:name/list?prefix=<p>`    → one level: folders and files
//! `GET /storage/:ns/:name/preview?path=<p>`   → the first 64 KiB, text when it is
//!
//! Drivers, by the resource's type:
//! - `s3` / `minio`: `{ bucket, region?, endpoint?, prefix?, access_key_id_ref?,
//!   secret_access_key_ref?, anonymous? }` — object_store (AWS)
//! - `gcs`: `{ bucket, prefix?, service_account_ref?, anonymous? }` — object_store (GCP)
//! - `azure`: `{ account, container, prefix?, access_key_ref? }` — object_store (Azure)
//! - `ftp`: `{ host, port?, user?, password_ref?, root? }` — suppaftp (plain FTP)
//!
//! Credentials are only ever secret references (`*_ref` → a Secret resource's
//! value), never read from the storage resource itself. There is deliberately no
//! local-filesystem driver: it would let a resource read any file on the pod.
//! Read only; signed-in callers only.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use flowr_storage::StorageBackend;
use object_store::{path::Path as OPath, ObjectStore, ObjectStoreExt};
use serde::Deserialize;
use serde_json::{json, Value};

const PREVIEW_BYTES: u64 = 64 * 1024;
const MAX_ENTRIES: usize = 1000;

struct St { storage: Arc<dyn StorageBackend> }

pub fn router(storage: Arc<dyn StorageBackend>) -> Router {
    Router::new()
        .route("/:ns/:name/list", get(list))
        .route("/:ns/:name/preview", get(preview))
        .with_state(Arc::new(St { storage }))
}

#[derive(Deserialize)]
struct Q { prefix: Option<String>, path: Option<String> }

fn err(code: StatusCode, msg: impl Into<String>) -> Response { (code, Json(json!({ "error": msg.into() }))).into_response() }

/// A Secret resource's value (`ns/name`, or `name` in default).
async fn secret(storage: &dyn StorageBackend, r: &str) -> Option<String> {
    let (ns, name) = r.split_once('/').unwrap_or(("default", r));
    let s = storage.resources().get(ns, name).await.ok()?;
    ["value", "token", "api_key", "password"].iter().find_map(|k| s.config.get(*k).and_then(|v| v.as_str()).map(str::to_string))
}

/// What a resource is to browse: an object store and its base prefix, or an FTP server.
enum Driver { Objects { store: Box<dyn ObjectStore>, base: String, location: String }, Ftp(FtpCfg) }
#[derive(Clone)]
struct FtpCfg { host: String, port: u16, user: String, password: String, root: String }

async fn driver(storage: &dyn StorageBackend, ns: &str, name: &str) -> Result<(String, Driver), Response> {
    let r = storage.resources().get(ns, name).await.map_err(|_| err(StatusCode::NOT_FOUND, format!("no resource {ns}/{name}")))?;
    let c = &r.config;
    let s = |k: &str| c.get(k).and_then(|v| v.as_str()).map(str::to_string).filter(|v| !v.is_empty());
    let b = |k: &str| c.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
    let base = s("prefix").map(|p| p.trim_matches('/').to_string()).unwrap_or_default();
    let typ = r.type_ref.to_lowercase();
    let need = |k: &str| s(k).ok_or_else(|| err(StatusCode::UNPROCESSABLE_ENTITY, format!("{ns}/{name}: its config has no {k}")));
    let sec = |k: &str| { let k = k.to_string(); async move { match s(&k) { Some(r) => secret(storage, &r).await, None => None } } };
    let bad = |e: object_store::Error| err(StatusCode::UNPROCESSABLE_ENTITY, format!("{ns}/{name}: {e}"));
    match typ.as_str() {
        "s3" | "minio" => {
            let bucket = need("bucket")?;
            let mut bld = object_store::aws::AmazonS3Builder::new().with_bucket_name(&bucket)
                .with_region(s("region").unwrap_or_else(|| "us-east-1".into()));
            if let Some(ep) = s("endpoint") { bld = bld.with_allow_http(ep.starts_with("http://")).with_endpoint(ep); }
            match (sec("access_key_id_ref").await, sec("secret_access_key_ref").await) {
                (Some(k), Some(v)) => bld = bld.with_access_key_id(k).with_secret_access_key(v),
                _ if b("anonymous") => bld = bld.with_skip_signature(true),
                _ => return Err(err(StatusCode::UNPROCESSABLE_ENTITY, format!("{ns}/{name}: set access_key_id_ref and secret_access_key_ref (Secrets), or anonymous: true for a public bucket"))),
            }
            Ok((typ, Driver::Objects { store: Box::new(bld.build().map_err(bad)?), location: format!("s3://{bucket}/{base}"), base }))
        }
        "gcs" => {
            let bucket = need("bucket")?;
            let mut bld = object_store::gcp::GoogleCloudStorageBuilder::new().with_bucket_name(&bucket);
            match sec("service_account_ref").await {
                Some(k) => bld = bld.with_service_account_key(k),
                None if b("anonymous") => bld = bld.with_skip_signature(true),
                None => return Err(err(StatusCode::UNPROCESSABLE_ENTITY, format!("{ns}/{name}: set service_account_ref (a Secret holding the key JSON), or anonymous: true"))),
            }
            Ok((typ, Driver::Objects { store: Box::new(bld.build().map_err(bad)?), location: format!("gs://{bucket}/{base}"), base }))
        }
        "azure" => {
            let (account, container) = (need("account")?, need("container")?);
            let key = sec("access_key_ref").await.ok_or_else(|| err(StatusCode::UNPROCESSABLE_ENTITY, format!("{ns}/{name}: set access_key_ref (a Secret)")))?;
            let bld = object_store::azure::MicrosoftAzureBuilder::new().with_account(&account).with_container_name(&container).with_access_key(key);
            Ok((typ, Driver::Objects { store: Box::new(bld.build().map_err(bad)?), location: format!("az://{account}/{container}/{base}"), base }))
        }
        "ftp" => {
            let host = need("host")?;
            let port = c.get("port").and_then(|v| v.as_u64()).unwrap_or(21) as u16;
            let user = s("user").unwrap_or_else(|| "anonymous".into());
            let password = sec("password_ref").await.unwrap_or_else(|| "anonymous@".into());
            let root = s("root").unwrap_or_else(|| "/".into());
            Ok((typ, Driver::Ftp(FtpCfg { host, port, user, password, root })))
        }
        other => Err(err(StatusCode::UNPROCESSABLE_ENTITY, format!("{ns}/{name} is a {other}: Files can browse s3, minio, gcs, azure and ftp"))),
    }
}

/// `a/b/` joined under the base prefix, as an object path.
fn under(base: &str, rel: &str) -> String {
    let rel = rel.trim_matches('/');
    match (base.is_empty(), rel.is_empty()) { (true, _) => rel.to_string(), (false, true) => base.to_string(), (false, false) => format!("{base}/{rel}") }
}
/// An object path made relative to the base prefix again.
fn rel_of(base: &str, full: &str) -> String {
    if base.is_empty() { full.to_string() } else { full.strip_prefix(base).unwrap_or(full).trim_start_matches('/').to_string() }
}
/// Guard against walking out of the root.
fn safe(p: &str) -> bool { !p.split('/').any(|s| s == "..") }

async fn list(State(st): State<Arc<St>>, principal: Option<axum::Extension<flowr_policy::Principal>>, Path((ns, name)): Path<(String, String)>, Query(q): Query<Q>) -> Response {
    if principal.is_none() { return err(StatusCode::UNAUTHORIZED, "sign in to browse files"); }
    let prefix = q.prefix.unwrap_or_default();
    if !safe(&prefix) { return err(StatusCode::BAD_REQUEST, "no .. in a path"); }
    let (typ, d) = match driver(&*st.storage, &ns, &name).await { Ok(x) => x, Err(r) => return r };
    match d {
        Driver::Objects { store, base, location } => {
            let at = under(&base, &prefix);
            let p = if at.is_empty() { None } else { Some(OPath::from(at.as_str())) };
            match store.list_with_delimiter(p.as_ref()).await {
                Ok(l) => {
                    let dirs: Vec<Value> = l.common_prefixes.iter().take(MAX_ENTRIES).map(|d| { let r = rel_of(&base, d.as_ref()); json!({ "name": r.rsplit('/').next().unwrap_or(&r), "path": format!("{r}/") }) }).collect();
                    let truncated = l.objects.len() > MAX_ENTRIES || l.common_prefixes.len() > MAX_ENTRIES;
                    let files: Vec<Value> = l.objects.iter().take(MAX_ENTRIES).map(|o| { let r = rel_of(&base, o.location.as_ref()); json!({ "name": r.rsplit('/').next().unwrap_or(&r), "path": r, "size": o.size, "modified": o.last_modified.to_rfc3339() }) }).collect();
                    Json(json!({ "driver": typ, "location": location, "prefix": prefix, "dirs": dirs, "files": files, "truncated": truncated })).into_response()
                }
                Err(e) => err(StatusCode::BAD_GATEWAY, format!("{typ}: {e}")),
            }
        }
        Driver::Ftp(cfg) => {
            let dir = format!("{}/{}", cfg.root.trim_end_matches('/'), prefix.trim_matches('/'));
            let c2 = cfg.clone();
            let res = tokio::task::spawn_blocking(move || -> Result<Vec<suppaftp::list::File>, String> {
                let mut f = suppaftp::FtpStream::connect((c2.host.as_str(), c2.port)).map_err(|e| e.to_string())?;
                f.get_ref().set_read_timeout(Some(std::time::Duration::from_secs(20))).ok();
                f.login(&c2.user, &c2.password).map_err(|e| e.to_string())?;
                let lines = f.list(Some(&dir)).map_err(|e| e.to_string())?;
                let _ = f.quit();
                Ok(lines.iter().filter_map(|l| suppaftp::list::File::try_from(l.as_str()).ok()).collect())
            }).await;
            match res {
                Ok(Ok(entries)) => {
                    let rel = prefix.trim_matches('/');
                    let join = |n: &str| if rel.is_empty() { n.to_string() } else { format!("{rel}/{n}") };
                    let dirs: Vec<Value> = entries.iter().filter(|e| e.is_directory()).take(MAX_ENTRIES).map(|e| json!({ "name": e.name(), "path": format!("{}/", join(e.name())) })).collect();
                    let files: Vec<Value> = entries.iter().filter(|e| e.is_file()).take(MAX_ENTRIES).map(|e| {
                        let m: chrono::DateTime<chrono::Utc> = e.modified().into();
                        json!({ "name": e.name(), "path": join(e.name()), "size": e.size(), "modified": m.to_rfc3339() })
                    }).collect();
                    Json(json!({ "driver": typ, "location": format!("ftp://{}:{}{}", cfg.host, cfg.port, cfg.root), "prefix": prefix, "dirs": dirs, "files": files, "truncated": entries.len() > MAX_ENTRIES })).into_response()
                }
                Ok(Err(e)) => err(StatusCode::BAD_GATEWAY, format!("ftp: {e}")),
                Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            }
        }
    }
}

async fn preview(State(st): State<Arc<St>>, principal: Option<axum::Extension<flowr_policy::Principal>>, Path((ns, name)): Path<(String, String)>, Query(q): Query<Q>) -> Response {
    if principal.is_none() { return err(StatusCode::UNAUTHORIZED, "sign in to read files"); }
    let path = q.path.unwrap_or_default();
    if path.is_empty() || !safe(&path) { return err(StatusCode::BAD_REQUEST, "path is a file, with no .."); }
    let (typ, d) = match driver(&*st.storage, &ns, &name).await { Ok(x) => x, Err(r) => return r };
    let (bytes, size): (Vec<u8>, u64) = match d {
        Driver::Objects { store, base, .. } => {
            let p = OPath::from(under(&base, &path).as_str());
            let size = match store.head(&p).await { Ok(m) => m.size, Err(e) => return err(StatusCode::BAD_GATEWAY, format!("{typ}: {e}")) };
            let end = size.min(PREVIEW_BYTES);
            if end == 0 { (Vec::new(), size) } else {
                match store.get_range(&p, 0..end).await { Ok(b) => (b.to_vec(), size), Err(e) => return err(StatusCode::BAD_GATEWAY, format!("{typ}: {e}")) }
            }
        }
        Driver::Ftp(cfg) => {
            let full = format!("{}/{}", cfg.root.trim_end_matches('/'), path.trim_start_matches('/'));
            let res = tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, u64), String> {
                use std::io::Read;
                let mut f = suppaftp::FtpStream::connect((cfg.host.as_str(), cfg.port)).map_err(|e| e.to_string())?;
                f.get_ref().set_read_timeout(Some(std::time::Duration::from_secs(20))).ok();
                f.login(&cfg.user, &cfg.password).map_err(|e| e.to_string())?;
                f.transfer_type(suppaftp::types::FileType::Binary).map_err(|e| e.to_string())?;
                let size = f.size(&full).map(|s| s as u64).unwrap_or(0);
                let mut buf = Vec::new();
                {
                    let mut r = f.retr_as_stream(&full).map_err(|e| e.to_string())?;
                    (&mut r).take(PREVIEW_BYTES).read_to_end(&mut buf).map_err(|e| e.to_string())?;
                    // A partial read: the server sees the data connection close early.
                    drop(r);
                }
                let _ = f.quit();
                Ok((buf, size))
            }).await;
            match res { Ok(Ok(x)) => x, Ok(Err(e)) => return err(StatusCode::BAD_GATEWAY, format!("ftp: {e}")), Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()) }
        }
    };
    let truncated = size > bytes.len() as u64;
    match std::str::from_utf8(&bytes) {
        Ok(t) => Json(json!({ "path": path, "size": size, "text": t, "truncated": truncated })).into_response(),
        // A cut mid-character at the 64 KiB edge is still text.
        Err(e) if e.error_len().is_none() && e.valid_up_to() > 0 => Json(json!({ "path": path, "size": size, "text": String::from_utf8_lossy(&bytes[..e.valid_up_to()]), "truncated": true })).into_response(),
        Err(_) => Json(json!({ "path": path, "size": size, "binary": true })).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_stay_under_the_root() {
        assert_eq!(under("", "a/b/"), "a/b");
        assert_eq!(under("base", ""), "base");
        assert_eq!(under("base", "/x/"), "base/x");
        assert_eq!(rel_of("base", "base/x/y.txt"), "x/y.txt");
        assert!(safe("a/b/c.txt") && !safe("a/../etc/passwd"));
    }
}
