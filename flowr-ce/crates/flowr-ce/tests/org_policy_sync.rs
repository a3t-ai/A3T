//! An org's policy files come from its own GitHub repo and are kept in A3T's
//! database: a sync takes a whole commit or none of it, a bad push or an
//! unreachable GitHub leaves the last good set enforced, a signed push syncs,
//! and only the org's admins (or platform security) may manage it.
//!
//! GitHub is a local stand-in serving the REST calls FlowR makes; the policy
//! gate is a stand-in for FlowR EE's compiler that refuses files saying BROKEN.

use std::collections::BTreeMap;
use std::sync::{atomic::{AtomicUsize, Ordering}, Arc, Mutex};

use axum::{extract::{Path, State}, http::{HeaderMap, StatusCode}, response::IntoResponse, routing::get, Json, Router};
use flowr_storage::StorageBackend;
use flowr_types::Resource;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

// ── A stand-in GitHub ──

#[derive(Default)]
struct Repo {
    commit: String,
    files: BTreeMap<String, String>,
    up: bool,
}
type Gh = Arc<Mutex<Repo>>;

fn authed(h: &HeaderMap) -> bool {
    h.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer gh-token")
}

async fn commit(State(g): State<Gh>, h: HeaderMap, Path((_o, _r, _ref)): Path<(String, String, String)>) -> axum::response::Response {
    let g = g.lock().unwrap();
    if !g.up { return StatusCode::SERVICE_UNAVAILABLE.into_response(); }
    if !authed(&h) { return StatusCode::UNAUTHORIZED.into_response(); }
    Json(json!({ "sha": g.commit })).into_response()
}

async fn tree(State(g): State<Gh>, h: HeaderMap, Path((_o, _r, sha)): Path<(String, String, String)>) -> axum::response::Response {
    let g = g.lock().unwrap();
    if !authed(&h) || sha != g.commit { return StatusCode::NOT_FOUND.into_response(); }
    let tree: Vec<Value> = g.files.iter().map(|(p, t)| json!({ "path": p, "type": "blob", "sha": blob_sha(p), "size": t.len() }))
        .chain([json!({ "path": "sec/policies", "type": "tree", "sha": "t1" })]).collect();
    Json(json!({ "sha": sha, "tree": tree, "truncated": false })).into_response()
}

async fn blob(State(g): State<Gh>, h: HeaderMap, Path((_o, _r, sha)): Path<(String, String, String)>) -> axum::response::Response {
    let g = g.lock().unwrap();
    if !authed(&h) { return StatusCode::UNAUTHORIZED.into_response(); }
    match g.files.iter().find(|(p, _)| blob_sha(p) == sha) {
        Some((_, t)) => t.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn blob_sha(path: &str) -> String {
    Sha256::digest(path.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>()[..40].to_string()
}

async fn serve(app: Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

// ── A stand-in for FlowR EE's compiler ──

#[derive(Default)]
struct Gate {
    reloads: AtomicUsize,
}

#[async_trait::async_trait]
impl flowr_ce::policy_gate::PolicyGate for Gate {
    async fn check(&self, _: &str, _: &str, _: Option<&str>) -> Result<(), String> { Ok(()) }
    async fn changed(&self) { self.reloads.fetch_add(1, Ordering::SeqCst); }
    async fn compile_org_files(&self, _org: &str, projects: &[String], files: &[(String, String)]) -> Result<Value, Vec<String>> {
        let errors: Vec<String> = files.iter().filter(|(_, t)| t.contains("BROKEN")).map(|(p, _)| format!("{p}: broken")).collect();
        if !errors.is_empty() { return Err(errors); }
        Ok(json!({ "files": files.iter().map(|(p, _)| json!({ "path": p })).collect::<Vec<_>>(), "projects": projects }))
    }
}

// ── FlowR, with a test sign-in: `x-user: <id>/<role,role>` ──

async fn flowr(storage: Arc<dyn StorageBackend>) -> String {
    let app = flowr_ce::org_policy::router(storage).layer(axum::middleware::from_fn(
        |mut req: axum::extract::Request, next: axum::middleware::Next| async move {
            let h = req.headers().get("x-user").and_then(|v| v.to_str().ok()).map(String::from);
            if let Some((id, roles)) = h.as_deref().and_then(|h| h.split_once('/')) {
                let roles = roles.split(',').filter(|r| !r.is_empty()).map(String::from).collect();
                req.extensions_mut().insert(flowr_policy::Principal::human(id, roles));
            }
            next.run(req).await
        }));
    serve(app).await
}

fn sign(secret: &str, body: &[u8]) -> String {
    let mut k = [0u8; 64];
    k[..secret.len()].copy_from_slice(secret.as_bytes());
    let pad = |b: u8| k.iter().map(|x| x ^ b).collect::<Vec<u8>>();
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(body).finalize();
    let mac = Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize();
    format!("sha256={}", mac.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_orgs_files_sync_from_its_repo_whole_commits_or_not_at_all() {
    // Storage with the platform tables: two orgs, acme's admin and member.
    let dir = tempfile::tempdir().unwrap();
    let storage = flowr_ce::init_storage(dir.path().to_str().unwrap()).await.unwrap();
    let db_path = dir.path().join("platform.lance");
    storage.resources().create(Resource::new("a3t-platform-db", "lancedb", json!({ "db_path": db_path.to_str().unwrap() })).with_namespace("a3t")).await.unwrap();
    let db = flowr_ce::platform_db::open_platform_db(storage.as_ref(), Some("a3t/a3t-platform-db"), "").await;
    let table = |t: &'static str, cols: &[&str]| {
        let schema: serde_json::Map<String, Value> = cols.iter().map(|c| (c.to_string(), json!("string"))).collect();
        let db = db.clone();
        async move { db.create_table(t, Some(&schema), "create_if_not_exists").await.unwrap(); }
    };
    table("orgs", &["id", "slug"]).await;
    table("org_members", &["org_id", "user_id", "role"]).await;
    table("projects", &["namespace", "owner_kind", "owner", "owner_id"]).await;
    db.insert("orgs", &[json!({ "id": "org-acme", "slug": "acme" }), json!({ "id": "org-globex", "slug": "globex" })]).await.unwrap();
    db.insert("org_members", &[
        json!({ "org_id": "org-acme", "user_id": "ann", "role": "securityadmin" }),
        json!({ "org_id": "org-acme", "user_id": "bob", "role": "member" }),
        json!({ "org_id": "org-globex", "user_id": "gil", "role": "admin" }),
    ]).await.unwrap();
    db.insert("projects", &[
        json!({ "namespace": "acme-crm", "owner_kind": "org", "owner": "acme", "owner_id": "org-acme" }),
        json!({ "namespace": "globex-web", "owner_kind": "org", "owner": "globex", "owner_id": "org-globex" }),
    ]).await.unwrap();
    for (name, value) in [("gh-token", "gh-token"), ("gh-hook", "hook-secret")] {
        storage.resources().create(Resource::new(name, "secret", json!({ "value": value })).with_namespace("acme-crm")).await.unwrap();
    }
    storage.resources().create(Resource::new("theirs", "secret", json!({ "value": "x" })).with_namespace("globex-web")).await.unwrap();

    let gh: Gh = Arc::new(Mutex::new(Repo { commit: "a".repeat(40), up: true, ..Default::default() }));
    {
        let mut g = gh.lock().unwrap();
        g.files.insert("sec/policies/models.yaml".into(), "kind: a3t/policy # v1".into());
        g.files.insert("sec/roles/lead.yaml".into(), "kind: a3t/role".into());
        g.files.insert("sec/README.md".into(), "not ours".into());
        g.files.insert("other/policies/x.yaml".into(), "outside the folder".into());
    }
    let gh_url = serve(Router::new()
        .route("/repos/:o/:r/commits/:ref", get(commit))
        .route("/repos/:o/:r/git/trees/:sha", get(tree))
        .route("/repos/:o/:r/git/blobs/:sha", get(blob))
        .with_state(gh.clone())).await;
    std::env::set_var("FLOWR_GITHUB_API", &gh_url);
    let gate = Arc::new(Gate::default());
    flowr_ce::policy_gate::install(gate.clone());
    let url = flowr(storage.clone()).await;
    let http = reqwest::Client::new();
    let call = |method: reqwest::Method, path: &str, user: &str, body: Option<Value>| {
        let mut r = http.request(method, format!("{url}{path}")).header("x-user", user);
        if let Some(b) = body { r = r.json(&b); }
        async move { let r = r.send().await.unwrap(); let s = r.status().as_u16(); (s, r.json::<Value>().await.unwrap_or(Value::Null)) }
    };
    use reqwest::Method as M;
    let source = json!({ "repo": "acme/security", "ref": "main", "folder": "sec", "mode": "live",
                         "token_secret": "acme-crm/gh-token", "webhook_secret": "acme-crm/gh-hook" });

    // Who may connect it: acme's security admin; not a member, not another org's admin.
    assert_eq!(call(M::PUT, "/security/orgs/acme/source", "bob/Operator", Some(source.clone())).await.0, 404);
    assert_eq!(call(M::PUT, "/security/orgs/acme/source", "gil/Admin", Some(source.clone())).await.0, 404);
    // A secret from another org's project is refused.
    let mut theirs = source.clone();
    theirs["token_secret"] = json!("globex-web/theirs");
    let (st, b) = call(M::PUT, "/security/orgs/acme/source", "ann/Viewer", Some(theirs)).await;
    assert_eq!(st, 400, "{b}");
    let (st, b) = call(M::PUT, "/security/orgs/acme/source", "ann/Viewer", Some(source.clone())).await;
    assert_eq!(st, 200, "{b}");
    assert!(!b.to_string().contains("hook-secret") && !b.to_string().contains("\"gh-token\""), "no secret values: {b}");

    // First sync: the folder's A3T files only.
    let (st, b) = call(M::POST, "/security/orgs/acme/sync", "ann/Viewer", None).await;
    assert_eq!(st, 200, "{b}");
    let (_, f) = call(M::GET, "/security/orgs/acme/files", "ann/Viewer", None).await;
    let paths: Vec<&str> = f["files"].as_array().unwrap().iter().map(|x| x["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["policies/models.yaml", "roles/lead.yaml"]);
    assert_eq!(f["projects"], json!(["acme-crm"]));
    assert_eq!(gate.reloads.load(Ordering::SeqCst), 1);
    let kept = flowr_ce::org_policy::all_org_files(storage.as_ref()).await;
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].0, "acme");

    // A push with one broken file: refused whole, the last good set stays.
    {
        let mut g = gh.lock().unwrap();
        g.commit = "b".repeat(40);
        g.files.insert("sec/policies/models.yaml".into(), "kind: a3t/policy # v2".into());
        g.files.insert("sec/policies/new.yaml".into(), "BROKEN".into());
    }
    let (st, b) = call(M::POST, "/security/orgs/acme/sync", "ann/Viewer", None).await;
    assert_eq!(st, 422, "{b}");
    assert_eq!(b["errors"], json!(["policies/new.yaml: broken"]));
    let (_, f) = call(M::GET, "/security/orgs/acme/files", "ann/Viewer", None).await;
    assert!(f["files"].to_string().contains("# v1") && !f["files"].to_string().contains("# v2"), "{f}");
    assert!(f["source"]["last_error"].as_str().unwrap().contains("bbbbbbb was not applied"), "{f}");

    // GitHub down: the kept set stays, the reason is recorded.
    gh.lock().unwrap().up = false;
    let (st, _) = call(M::POST, "/security/orgs/acme/sync", "ann/Viewer", None).await;
    assert_eq!(st, 502);
    assert_eq!(flowr_ce::org_policy::all_org_files(storage.as_ref()).await[0].2.len(), 2);
    gh.lock().unwrap().up = true;

    // A fixed push arrives by webhook: an unsigned or wrongly signed one is refused,
    // a signed one syncs (the file it dropped is no longer kept).
    {
        let mut g = gh.lock().unwrap();
        g.commit = "c".repeat(40);
        g.files.remove("sec/policies/new.yaml");
        g.files.remove("sec/roles/lead.yaml");
    }
    let event = serde_json::to_vec(&json!({ "ref": "refs/heads/main", "after": "c".repeat(40), "repository": { "full_name": "acme/security" } })).unwrap();
    let hook = |sig: String| http.post(format!("{url}/webhook/_a3t/github")).header("x-github-event", "push")
        .header("x-hub-signature-256", sig).body(event.clone()).send();
    assert_eq!(hook(sign("wrong", &event)).await.unwrap().status().as_u16(), 401);
    let r: Value = hook(sign("hook-secret", &event)).await.unwrap().json().await.unwrap();
    assert_eq!(r["synced"][0]["org"], "acme", "{r}");
    assert_eq!(r["synced"][0]["changed"], true, "{r}");
    let (_, f) = call(M::GET, "/security/orgs/acme/files", "ann/Viewer", None).await;
    let paths: Vec<&str> = f["files"].as_array().unwrap().iter().map(|x| x["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["policies/models.yaml"]);
    assert!(f["files"].to_string().contains("# v2"));
    assert_eq!(f["source"]["last_commit"], "c".repeat(40));
    assert_eq!(f["source"]["last_error"], "");

    // Disconnecting the repo keeps enforcing the files; removing the files stops it.
    assert_eq!(call(M::DELETE, "/security/orgs/acme/source", "ann/Viewer", None).await.0, 200);
    assert_eq!(flowr_ce::org_policy::all_org_files(storage.as_ref()).await.len(), 1);
    assert_eq!(call(M::DELETE, "/security/orgs/acme/files", "bob/Operator", None).await.0, 404);
    assert_eq!(call(M::DELETE, "/security/orgs/acme/files", "root/SecurityAdmin", None).await.0, 200);
    assert!(flowr_ce::org_policy::all_org_files(storage.as_ref()).await.is_empty());
}
