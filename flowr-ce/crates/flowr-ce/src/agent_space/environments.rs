//! Environments as data (AC-12, D4): per project, a row saying what each environment is:
//! whether it's production, its runtime and gateway pools, where its memory goes, and how
//! many approvals and how much evidence a change to it needs. A project with no rows has
//! the defaults: `dev` and `qa` send memory to the non-prod ADB and need nothing; `prod`
//! uses the org's ADB and needs one approval and one piece of evidence.

use super::*;

pub const ENVIRONMENTS: &str = "project_environments";
const COLUMNS: [&str; 10] = ["project", "name", "production", "runtime_pool", "gateway_pool", "memory_resource", "required_approvals", "required_evidence", "updated_by", "updated_at"];

/// The non-prod ADB's resource (`A3T_NONPROD_ADB_RESOURCE`).
pub fn non_prod_resource() -> Option<String> {
    std::env::var("A3T_NONPROD_ADB_RESOURCE").ok().filter(|v| !v.is_empty())
}

/// The non-prod pod's Flight URL, as branches are placed on it (`A3T_NONPROD_ADB_FLIGHT`).
pub fn non_prod_flight() -> Option<String> {
    std::env::var("A3T_NONPROD_ADB_FLIGHT").ok().filter(|v| !v.is_empty())
}

pub(super) fn routes() -> Router<Arc<dyn StorageBackend>> {
    Router::new()
        .route("/projects/:project/environments", get(list))
        .route("/projects/:project/environments/:name", axum::routing::put(put_one).delete(delete_one))
}

fn default_row(project: &str, name: &str, production: bool) -> Value {
    let need = if production { "1" } else { "0" };
    let memory = if production { org_adb("") } else { non_prod_resource().unwrap_or_else(|| org_adb("")) };
    json!({
        "project": project, "name": name, "production": production.to_string(),
        "runtime_pool": "", "gateway_pool": "", "memory_resource": memory,
        "required_approvals": need, "required_evidence": need,
    })
}

fn defaults(project: &str) -> Vec<Value> {
    vec![default_row(project, "dev", false), default_row(project, "qa", false), default_row(project, "prod", true)]
}

/// A project's environments: its rows, or the defaults when it has none.
pub async fn of_project(s: &dyn StorageBackend, project: &str) -> Result<Vec<Value>, String> {
    let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
    let schema: Map<String, Value> = COLUMNS.iter().map(|c| (c.to_string(), json!("string"))).collect();
    db.create_table(ENVIRONMENTS, Some(&schema), "create_if_not_exists").await?;
    let rows = query_platform_table(s, ENVIRONMENTS, Some(format!("project = {}", sql_quote(project))), 1000).await?;
    Ok(if rows.is_empty() { defaults(project) } else { rows })
}

pub(super) async fn find(s: &dyn StorageBackend, project: &str, name: &str) -> Result<Value, Response> {
    let all = of_project(s, project).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    all.into_iter().find(|e| col(e, "name") == name).ok_or_else(|| err(StatusCode::NOT_FOUND, format!("project {project:?} has no environment {name:?}")))
}

pub(super) fn is_production(env: &Value) -> bool {
    col(env, "production") == "true"
}

pub(super) fn needs_approval(env: &Value) -> bool {
    col(env, "required_approvals").parse::<u64>().unwrap_or(0) > 0 || col(env, "required_evidence").parse::<u64>().unwrap_or(0) > 0
}

async fn list(State(s): St, p: P, Path(project): Path<String>) -> Response {
    both(async {
        if !crate::access::can_read_item(&who(p), ItemKind::Agent, &project, "", "").await {
            return Err(err(StatusCode::NOT_FOUND, format!("no project {project:?}")));
        }
        ok(json!({ "environments": of_project(s.as_ref(), &project).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))? }))
    }.await)
}

/// `{production?, runtime_pool?, gateway_pool?, memory_resource?, required_approvals?,
/// required_evidence?}`. The first write fixes the defaults as the project's rows.
async fn put_one(State(s): St, p: P, Path((project, name)): Path<(String, String)>, Json(b): Json<Value>) -> Response {
    both(async {
        let principal = who(p);
        let s = s.as_ref();
        crate::access::rest_write(&principal, &project).await.map_err(IntoResponse::into_response)?;
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(err(StatusCode::BAD_REQUEST, format!("environment {name:?}: letters, digits and '-'")));
        }
        let all = of_project(s, &project).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let mut row = all.iter().find(|e| col(e, "name") == name).cloned().unwrap_or_else(|| default_row(&project, &name, false));
        for k in ["runtime_pool", "gateway_pool", "memory_resource"] {
            if let Some(v) = b.get(k).and_then(|v| v.as_str()) {
                row[k] = json!(v);
            }
        }
        for k in ["production", "required_approvals", "required_evidence"] {
            if let Some(v) = b.get(k) {
                row[k] = json!(v.as_str().map(String::from).unwrap_or_else(|| v.to_string()));
            }
        }
        // Who changes what production is, or relaxes its rule, is a production decision.
        if is_production(&row) || all.iter().any(|e| col(e, "name") == name && is_production(e)) {
            allowed(&principal, flowr_policy::actions::DEPLOY_PROD, &project).await?;
        }
        let stamp = |mut r: Value| {
            r["updated_by"] = json!(principal.id());
            r["updated_at"] = json!(chrono::Utc::now().to_rfc3339());
            r
        };
        let mut rows: Vec<Value> = all.into_iter().filter(|e| col(e, "name") != name).map(&stamp).collect();
        let row = stamp(row);
        rows.push(row.clone());
        let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
        db.upsert(ENVIRONMENTS, &["project", "name"], &rows).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        ok(row)
    }.await)
}

async fn delete_one(State(s): St, p: P, Path((project, name)): Path<(String, String)>) -> Response {
    both(async {
        let principal = who(p);
        let s = s.as_ref();
        crate::access::rest_write(&principal, &project).await.map_err(IntoResponse::into_response)?;
        let env = find(s, &project, &name).await?;
        if is_production(&env) {
            allowed(&principal, flowr_policy::actions::DEPLOY_PROD, &project).await?;
        }
        let all = of_project(s, &project).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let db = open_platform_db(s, Some(PLATFORM_DB_RESOURCE), DEFAULT_PLATFORM_DB_PATH).await;
        // The defaults become rows first, so removing one doesn't bring the others back.
        let keep: Vec<Value> = all.into_iter().filter(|e| col(e, "name") != name).collect();
        db.upsert(ENVIRONMENTS, &["project", "name"], &keep).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        db.delete(ENVIRONMENTS, &format!("project = {} AND name = {}", sql_quote(&project), sql_quote(&name))).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        ok(json!({ "deleted": true }))
    }.await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_gate_prod_only() {
        let rows = defaults("acme");
        let names: Vec<String> = rows.iter().map(|r| col(r, "name")).collect();
        assert_eq!(names, ["dev", "qa", "prod"]);
        assert!(!is_production(&rows[0]) && !needs_approval(&rows[0]));
        assert!(!is_production(&rows[1]) && !needs_approval(&rows[1]));
        assert!(is_production(&rows[2]) && needs_approval(&rows[2]));
    }
}
