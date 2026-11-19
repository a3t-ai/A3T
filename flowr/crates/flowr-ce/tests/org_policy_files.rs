//! An org's policy files live in A3T's database as files: saved only when the
//! org's enforced files still work together, every version kept, each file
//! switched on or off, and only the org's admins and security admins (or
//! platform security) may manage them.
//!
//! The policy gate is a stand-in for FlowR EE's compiler that refuses files
//! saying BROKEN, and a file saying `NEEDS:<path>` without that file in the set
//! (a rule naming a role defined in another file).

use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};

use flowr_storage::StorageBackend;
use flowr_types::Resource;
use serde_json::{json, Value};

async fn serve(app: axum::Router) -> String {
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
        let mut errors: Vec<String> = files.iter().filter(|(_, t)| t.contains("BROKEN")).map(|(p, _)| format!("{p}: broken")).collect();
        for (p, t) in files {
            if let Some(need) = t.split("NEEDS:").nth(1).and_then(|r| r.split_whitespace().next()) {
                if !files.iter().any(|(q, _)| q == need) { errors.push(format!("{p}: needs {need}")); }
            }
        }
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

#[tokio::test(flavor = "multi_thread")]
async fn an_orgs_policy_files_are_kept_versioned_and_switched() {
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
    let file = |p: &str| format!("/security/orgs/acme/file?path={p}");
    let body = |c: &str| Some(json!({ "content": c }));
    let enforced = || async { flowr_ce::org_policy::all_org_files(storage.as_ref()).await.into_iter()
        .find(|(o, _, _)| o == "acme").map(|(_, _, f)| f.into_iter().map(|(p, _)| p).collect::<Vec<_>>()).unwrap_or_default() };

    // acme's security admin writes a role, then a rule that names it.
    let (st, b) = call(M::PUT, &file("roles/lead.yaml"), "ann/Viewer", body("kind: a3t/role")).await;
    assert_eq!((st, b["version"].as_str()), (200, Some("v1")), "{b}");
    let (st, b) = call(M::PUT, &file("policies/models.yaml"), "ann/Viewer", body("kind: a3t/policy # v1 NEEDS:roles/lead.yaml")).await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(enforced().await, ["policies/models.yaml", "roles/lead.yaml"]);
    assert_eq!(gate.reloads.load(Ordering::SeqCst), 2);
    // Unchanged text: no new version, no reload.
    let (_, b) = call(M::PUT, &file("roles/lead.yaml"), "ann/Viewer", body("kind: a3t/role")).await;
    assert_eq!(b["changed"], false);
    assert_eq!(gate.reloads.load(Ordering::SeqCst), 2);

    // A broken file, or one naming a role nobody defined, is refused: nothing saved.
    let (st, b) = call(M::PUT, &file("policies/new.yaml"), "ann/Viewer", body("BROKEN")).await;
    assert_eq!(st, 422, "{b}");
    assert_eq!(b["errors"], json!(["policies/new.yaml: broken"]));
    let (st, _) = call(M::PUT, &file("policies/x.yaml"), "ann/Viewer", body("NEEDS:roles/nobody.yaml")).await;
    assert_eq!(st, 422);
    let (_, f) = call(M::GET, "/security/orgs/acme/files", "ann/Viewer", None).await;
    assert_eq!(f["files"].as_array().unwrap().len(), 2, "{f}");

    // A draft is checked without saving it.
    let (st, v) = call(M::POST, "/security/orgs/acme/validate", "ann/Viewer", Some(json!({ "files": [{ "path": "policies/d.yaml", "content": "BROKEN" }] }))).await;
    assert_eq!((st, &v["ok"]), (200, &json!(false)), "{v}");
    let (_, v) = call(M::POST, "/security/orgs/acme/validate", "ann/Viewer", Some(json!({ "removed": ["roles/lead.yaml"] }))).await;
    assert_eq!(v["ok"], false, "removing the role the rule needs: {v}");
    let (_, v) = call(M::POST, "/security/orgs/acme/validate", "ann/Viewer", Some(json!({}))).await;
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["checked"], json!(["policies/models.yaml", "roles/lead.yaml"]));

    // A new version; every version is kept.
    let (_, b) = call(M::PUT, &file("policies/models.yaml"), "ann/Viewer", body("kind: a3t/policy # v2 NEEDS:roles/lead.yaml")).await;
    assert_eq!(b["version"], "v2");
    let (_, h) = call(M::GET, &file("policies/models.yaml"), "ann/Viewer", None).await;
    assert_eq!(h["file"]["version"], "v2");
    let versions: Vec<(&str, &str)> = h["history"].as_array().unwrap().iter().map(|x| (x["version"].as_str().unwrap(), x["action"].as_str().unwrap())).collect();
    assert_eq!(versions, [("v2", "save"), ("v1", "create")]);
    assert!(h["history"][1]["content"].as_str().unwrap().contains("# v1"));
    assert_eq!(h["history"][0]["by"], "ann");

    // The role can't go while the rule needs it: not deleted, not switched off alone.
    assert_eq!(call(M::DELETE, &file("roles/lead.yaml"), "ann/Viewer", None).await.0, 422);
    let (st, _) = call(M::PUT, "/security/orgs/acme/files/enforced", "ann/Viewer", Some(json!({ "enforced": ["policies/models.yaml"] }))).await;
    assert_eq!(st, 422);
    // The rule switched off: kept, listed, not enforced; the switch is in its history.
    let (st, b) = call(M::PUT, "/security/orgs/acme/files/enforced", "ann/Viewer", Some(json!({ "enforced": ["roles/lead.yaml"] }))).await;
    assert_eq!((st, &b["switched"]), (200, &json!(1)), "{b}");
    assert_eq!(enforced().await, ["roles/lead.yaml"]);
    let (_, f) = call(M::GET, "/security/orgs/acme/files", "ann/Viewer", None).await;
    let state: Vec<(&str, bool)> = f["files"].as_array().unwrap().iter().map(|x| (x["path"].as_str().unwrap(), x["enforced"].as_bool().unwrap())).collect();
    assert_eq!(state, [("policies/models.yaml", false), ("roles/lead.yaml", true)]);
    let (_, h) = call(M::GET, &file("policies/models.yaml"), "ann/Viewer", None).await;
    assert_eq!(h["history"][0]["action"], "off");
    // Edited while off: checked as if on, saved, still off.
    let (st, b) = call(M::PUT, &file("policies/models.yaml"), "ann/Viewer", body("kind: a3t/policy # v3 NEEDS:roles/lead.yaml")).await;
    assert_eq!((st, &b["enforced"]), (200, &json!(false)), "{b}");
    assert_eq!(enforced().await, ["roles/lead.yaml"]);
    // Back on.
    let (st, _) = call(M::PUT, "/security/orgs/acme/files/enforced", "ann/Viewer", Some(json!({ "enforced": ["policies/models.yaml", "roles/lead.yaml"] }))).await;
    assert_eq!(st, 200);
    assert_eq!(enforced().await, ["policies/models.yaml", "roles/lead.yaml"]);

    // Who may: acme's admins and security admins, and platform security. Not a
    // member, not another org's admin; and only orgs that exist.
    assert_eq!(call(M::PUT, &file("access/a.yaml"), "bob/Operator", body("kind: a3t/access")).await.0, 404);
    assert_eq!(call(M::PUT, &file("access/a.yaml"), "gil/Admin", body("kind: a3t/access")).await.0, 404);
    assert_eq!(call(M::GET, "/security/orgs/acme/files", "bob/Operator", None).await.0, 404);
    assert_eq!(call(M::PUT, &file("access/a.yaml"), "root/SecurityAdmin", body("kind: a3t/access")).await.0, 200);
    assert_eq!(call(M::PUT, "/security/orgs/nope/file?path=access/a.yaml", "root/SecurityAdmin", body("kind: a3t/access")).await.0, 404);
    // Only policies/, roles/ and access/ files with plain names.
    for bad in ["README.md", "policies/../x.yaml", "other/x.yaml", "policies/a b.yaml"] {
        assert_eq!(call(M::PUT, &file(bad), "ann/Viewer", body("kind: a3t/policy")).await.0, 400, "{bad}");
    }

    // Removing: the rule first, then its role; their versions stay readable.
    assert_eq!(call(M::DELETE, &file("policies/models.yaml"), "ann/Viewer", None).await.0, 200);
    assert_eq!(call(M::DELETE, &file("roles/lead.yaml"), "ann/Viewer", None).await.0, 200);
    assert_eq!(enforced().await, ["access/a.yaml"]);
    let (st, h) = call(M::GET, &file("policies/models.yaml"), "ann/Viewer", None).await;
    assert_eq!(st, 200);
    assert!(h["file"].is_null());
    assert_eq!(h["history"][0]["action"], "delete");
    assert_eq!(h["history"].as_array().unwrap().len(), 6, "{h}");
    // Made again: its numbering goes on, so a version label always means one text.
    let (_, b) = call(M::PUT, &file("policies/models.yaml"), "ann/Viewer", body("kind: a3t/policy # v4")).await;
    assert_eq!(b["version"], "v4", "{b}");
}
