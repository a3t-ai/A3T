//! call:adb handler — runs AQL against ADB (Agent Database) agent memory
//!
//! In CNCF syntax: `call: adb` with `with:` containing `query` (AQL), optional
//! `variables`, and the ADB server from `_resource` (an `adb` resource: `config.url`,
//! optional `config.flight_url`) or `url`. `call:adb` talks to ADB over **Arrow
//! Flight** (ADB stage G): AQL is `do_action("aql")` with `POST /query`'s JSON in and
//! out; this module is also the Flight client for ADB platform tables. ADB over MCP
//! stays JSON over HTTP. `call:clawdb` is an alias (the resource type ADB replaces).

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::info;

use swf_parser::{NamedTask, Task};
use swf_runtime::context::ExecutionContext;
use swf_runtime::error::{RuntimeError, Result};
use swf_runtime::traits::{TaskHandler, TaskResult};

pub struct AdbHandler {
    task_type: &'static str,
}

impl AdbHandler {
    pub fn new(task_type: &'static str) -> Self {
        Self { task_type }
    }
}

#[async_trait]
impl TaskHandler for AdbHandler {
    fn task_type(&self) -> &str {
        self.task_type
    }

    async fn execute(
        &self,
        task: &NamedTask,
        _input: Value,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let with = match &task.task {
            Task::Call(c) => c.with.clone().unwrap_or(json!({})),
            _ => return Err(RuntimeError::task_failed(&task.name, "expected call task")),
        };
        let query = with.get("query").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if query.is_empty() {
            return Err(RuntimeError::task_failed(&task.name, "call:adb requires 'query' (AQL) in with"));
        }

        // A project's flow reaches its own memory only (platform flows are A3T's own code).
        let guarded = flowr_policy::namespace::namespace_guard().enforcing()
            && !flowr_policy::namespace::PLATFORM_NAMESPACES.contains(&ctx.namespace.as_str());
        if guarded && with.get("url").is_some() {
            return Err(RuntimeError::task_failed(&task.name,
                "call:adb in a project names an adb resource (`_resource`), not a url"));
        }
        // No silent default: a missing resource is an error, not a call to localhost.
        let ep = if let Some(u) = with.get("url").and_then(|v| v.as_str()) {
            adb_endpoint(u, with.get("flight_url").and_then(|v| v.as_str()))
        } else if let Some(r) = with.get("_resource").and_then(|v| v.as_str()) {
            // A run on a branch uses the same ADB: its namespaces become `<ns>@<branch>`.
            let storage = ctx
                .storage
                .as_ref()
                .and_then(|s| s.downcast_ref::<std::sync::Arc<dyn flowr_storage::StorageBackend>>())
                .ok_or_else(|| RuntimeError::task_failed(&task.name, "no storage to resolve the ADB resource"))?;
            resolve_adb(storage.as_ref(), r)
                .await
                .map_err(|e| RuntimeError::task_failed(&task.name, &e))?
        } else {
            return Err(RuntimeError::task_failed(&task.name, "call:adb requires '_resource' (an adb resource) or 'url'"));
        };

        if guarded {
            let named = with.get("_resource").and_then(|v| v.as_str()).unwrap_or_default();
            let org = crate::agent_space::org_adb("");
            let storage = ctx.storage.as_ref()
                .and_then(|s| s.downcast_ref::<std::sync::Arc<dyn flowr_storage::StorageBackend>>());
            // The shared org ADB — by name, or by a project resource pointing at the same server.
            let shared = named == org || match storage {
                Some(st) => resolve_adb(st.as_ref(), &org).await.map(|o| o.http == ep.http || o.flight == ep.flight).unwrap_or(false),
                None => false,
            };
            let own_resource = named.split_once('/').map(|(ns, _)| ns == ctx.namespace).unwrap_or(false);
            if !shared && !own_resource {
                return Err(RuntimeError::task_failed(&task.name, format!(
                    "a flow in project '{}' uses its own adb resources (or the org's memory); '{named}' is another project's",
                    ctx.namespace)));
            }
            if shared {
                project_namespaces_only(&query, &ctx.namespace).map_err(|e| RuntimeError::task_failed(&task.name, e))?;
            }
        }

        // AC-6: a run on a memory branch reads and writes that branch.
        let mut variables = with.get("variables").cloned();
        let query = match ctx.principal.memory_branch() {
            Some(b) => {
                let (q, vars) = on_branch(&query, b).map_err(|e| RuntimeError::task_failed(&task.name, e))?;
                // A namespace passed in a variable: the branch's too.
                for v in vars {
                    if let Some(slot) = variables.as_mut().and_then(|m| m.get_mut(&v)).filter(|x| x.is_string()) {
                        let ns = slot.as_str().unwrap_or_default().to_string();
                        if !ns.contains('@') {
                            *slot = serde_json::Value::String(format!("{ns}@{b}"));
                        }
                    }
                }
                q
            }
            None => query,
        };
        info!(task = %task.name, flight = %ep.flight, "calling ADB");
        let result = adb_query(&ep, &query, variables.as_ref())
            .await
            .map_err(|e| RuntimeError::Communication(e))?;
        Ok(TaskResult::new(result))
    }
}

/// The NAMESPACEs an AQL statement names (its string literals after `NAMESPACE`).
pub fn named_namespaces(aql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut quoted, mut escaped) = (false, false);
    let (mut word, mut lit) = (String::new(), String::new());
    let mut after_ns = false;
    for c in aql.chars() {
        if quoted {
            if escaped { escaped = false; lit.push(c); }
            else if c == '\\' { escaped = true; }
            else if c == '"' {
                quoted = false;
                if after_ns { out.push(std::mem::take(&mut lit)); }
                lit.clear();
                after_ns = false;
            } else { lit.push(c); }
            continue;
        }
        if c.is_alphanumeric() || c == '_' || c == '$' { word.push(c); continue; }
        if !word.is_empty() {
            after_ns = word.eq_ignore_ascii_case("NAMESPACE");
            word.clear();
        }
        if c == '"' { quoted = true; } else if !c.is_whitespace() { after_ns = false; }
    }
    out
}

/// On the shared org ADB a project's flow names its NAMESPACE, and only its own:
/// the project's (`<project>`) or one of its agents' spaces (`<project>--<agent>`).
pub fn project_namespaces_only(aql: &str, project: &str) -> std::result::Result<(), String> {
    let named = named_namespaces(aql);
    if named.is_empty() {
        return Err(format!("on the org's memory a query names its NAMESPACE: \"{project}\" or \"{project}--<agent>\""));
    }
    let own = |n: &str| n == project || n.strip_prefix(project).is_some_and(|rest| rest.starts_with("--"));
    match named.iter().find(|n| !own(n)) {
        Some(n) => Err(format!("NAMESPACE \"{n}\" is not project '{project}''s memory")),
        None => Ok(()),
    }
}

/// AC-6: `aql` as a run on memory branch `branch` sends it (AQL v0.13: a branch is the
/// namespace `<ns>@<branch>`): every namespace it names — after `NAMESPACE` (a literal,
/// each of a `("a", "b")` list, or a `$variable`, returned for the caller to rewrite in
/// the variables) and after `USE` — becomes `<ns>@<branch>`. A write with no namespace is
/// refused (it would reach main); a namespace of another branch too. A read with no
/// namespace, and topic statements (topics aren't on branches), stay as they are.
pub fn on_branch(aql: &str, branch: &str) -> std::result::Result<(String, Vec<String>), String> {
    let suffix = format!("@{branch}");
    let chars: Vec<(usize, char)> = aql.char_indices().collect();
    // Tokens outside strings: (upper-cased word or the literal's inner text, kind, start, end).
    #[derive(PartialEq)]
    enum K { Word, Str, Var, Open, Close, Other }
    let mut toks: Vec<(String, K, usize, usize)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '"' {
            let mut j = i + 1;
            let mut inner = String::new();
            while j < chars.len() {
                let (_, d) = chars[j];
                if d == '\\' && j + 1 < chars.len() {
                    inner.push(d);
                    inner.push(chars[j + 1].1);
                    j += 2;
                    continue;
                }
                if d == '"' {
                    break;
                }
                inner.push(d);
                j += 1;
            }
            let end = chars.get(j).map(|(p, _)| *p).unwrap_or(aql.len());
            toks.push((inner, K::Str, pos, end)); // end = the closing quote's position
            i = j + 1;
        } else if c == '{' {
            // `{name}`: a variable.
            let j = (i + 1..chars.len()).find(|&j| chars[j].1 == '}').unwrap_or(chars.len() - 1);
            let end = chars.get(j + 1).map(|(p, _)| *p).unwrap_or(aql.len());
            toks.push((aql[pos + 1..chars[j].0].to_string(), K::Var, pos, end));
            i = j + 1;
        } else if c == '$' || c.is_alphanumeric() || c == '_' {
            // A word, or `$name`.
            let mut j = i + 1;
            while j < chars.len() && (chars[j].1.is_alphanumeric() || chars[j].1 == '_' || chars[j].1 == '-') {
                j += 1;
            }
            let end = chars.get(j).map(|(p, _)| *p).unwrap_or(aql.len());
            let w = &aql[pos..end];
            match w.strip_prefix('$') {
                Some(v) => toks.push((v.to_string(), K::Var, pos, end)),
                None => toks.push((w.to_ascii_uppercase(), K::Word, pos, end)),
            }
            i = j;
        } else {
            toks.push((c.to_string(), match c { '(' => K::Open, ')' => K::Close, _ => K::Other }, pos, pos + c.len_utf8()));
            i += 1;
        }
    }
    let first = toks.iter().find(|t| t.1 == K::Word).map(|t| t.0.as_str()).unwrap_or("");
    let second = toks.iter().filter(|t| t.1 == K::Word).nth(1).map(|t| t.0.as_str()).unwrap_or("");
    // Topics aren't on branches: a topic statement goes as it is.
    if first == "PUBLISH" || first == "UNSUBSCRIBE" || (first == "SUBSCRIBE" && second == "TOPIC") {
        return Ok((aql.to_string(), Vec::new()));
    }
    // Insert `@branch` before each named namespace literal's closing quote.
    let mut inserts: Vec<usize> = Vec::new();
    let mut vars: Vec<String> = Vec::new();
    let mut named = false;
    let name = |t: &(String, K, usize, usize), inserts: &mut Vec<usize>| -> std::result::Result<(), String> {
        if let Some((_, b)) = t.0.rsplit_once('@') {
            return if b == branch { Ok(()) } else { Err(format!("this run is on memory branch {branch:?} and can't reach branch {b:?}")) };
        }
        inserts.push(t.3);
        Ok(())
    };
    let mut k = 0;
    while k < toks.len() {
        let t = &toks[k];
        if t.1 == K::Word && (t.0 == "NAMESPACE" || t.0 == "USE") {
            named = true;
            match toks.get(k + 1) {
                Some(n) if n.1 == K::Str => name(n, &mut inserts)?,
                Some(n) if n.1 == K::Var => vars.push(n.0.clone()),
                Some(n) if n.1 == K::Open => {
                    let mut m = k + 2;
                    while let Some(x) = toks.get(m).filter(|x| x.1 != K::Close) {
                        if x.1 == K::Str {
                            name(x, &mut inserts)?;
                        }
                        m += 1;
                    }
                }
                _ => return Err("NAMESPACE names nothing".into()),
            }
        }
        k += 1;
    }
    if !named && ["STORE", "UPDATE", "FORGET", "LINK", "UNLINK"].contains(&first) {
        return Err(format!("this run is on memory branch {branch:?}: a memory write must name its NAMESPACE, or it would reach main"));
    }
    let mut out = aql.to_string();
    inserts.sort_unstable();
    for at in inserts.into_iter().rev() {
        out.insert_str(at, &suffix);
    }
    Ok((out, vars))
}

#[cfg(test)]
mod namespace_guard_tests {
    use super::*;

    #[test]
    fn finds_every_named_namespace() {
        assert_eq!(named_namespaces(r#"RECALL FROM SEMANTIC NAMESPACE "acme-web" LIMIT 5"#), vec!["acme-web"]);
        assert_eq!(named_namespaces(r#"STORE INTO EPISODIC NAMESPACE "a--bot" (content = "NAMESPACE \"x\"")"#), vec!["a--bot"]);
        assert!(named_namespaces(r#"RECALL FROM SEMANTIC WHERE x = "NAMESPACE""#).is_empty());
    }

    #[test]
    fn a_project_reaches_only_its_own_memory() {
        assert!(project_namespaces_only(r#"RECALL FROM SEMANTIC NAMESPACE "acme" LIMIT 1"#, "acme").is_ok());
        assert!(project_namespaces_only(r#"RECALL FROM SEMANTIC NAMESPACE "acme--support" LIMIT 1"#, "acme").is_ok());
        assert!(project_namespaces_only(r#"RECALL FROM SEMANTIC NAMESPACE "acme2" LIMIT 1"#, "acme").is_err());
        assert!(project_namespaces_only(r#"RECALL FROM SEMANTIC NAMESPACE "other--bot" LIMIT 1"#, "acme").is_err());
        assert!(project_namespaces_only(r#"RECALL FROM SEMANTIC LIMIT 1"#, "acme").is_err());
    }
}

#[cfg(test)]
mod on_branch_tests {
    use super::on_branch;

    #[test]
    fn a_branch_run_names_the_branch_namespace() {
        let b = |q: &str| on_branch(q, "try").map(|(s, _)| s);
        assert_eq!(b(r#"STORE INTO SEMANTIC (key = "NAMESPACE x", v = 1) NAMESPACE "crm" TTL 1h"#).unwrap(), r#"STORE INTO SEMANTIC (key = "NAMESPACE x", v = 1) NAMESPACE "crm@try" TTL 1h"#);
        assert_eq!(on_branch(r#"RECALL FROM SEMANTIC NAMESPACE $ns LIMIT 1"#, "try").unwrap(), (r#"RECALL FROM SEMANTIC NAMESPACE $ns LIMIT 1"#.to_string(), vec!["ns".to_string()]));
        assert_eq!(b(r#"RECALL FROM SEMANTIC NAMESPACE "crm@try""#).unwrap(), r#"RECALL FROM SEMANTIC NAMESPACE "crm@try""#);
        assert!(b(r#"RECALL FROM SEMANTIC NAMESPACE "crm@other""#).is_err());
        assert_eq!(b(r#"RECALL FROM SEMANTIC LIKE $q NAMESPACE ("crm", "kb")"#).unwrap(), r#"RECALL FROM SEMANTIC LIKE $q NAMESPACE ("crm@try", "kb@try")"#);
        assert_eq!(b(r#"USE "crm"; STORE INTO EPISODIC (e = 1)"#).unwrap(), r#"USE "crm@try"; STORE INTO EPISODIC (e = 1)"#);
        assert!(b(r#"STORE INTO WORKING (key = "k")"#).is_err());
        assert_eq!(b(r#"RECALL FROM WORKING"#).unwrap(), r#"RECALL FROM WORKING"#);
        assert_eq!(b(r#"LINK FROM SEMANTIC WHERE key = "a" INTO SEMANTIC WHERE key = "b" TYPE "knows" NAMESPACE "crm""#).unwrap(),
            r#"LINK FROM SEMANTIC WHERE key = "a" INTO SEMANTIC WHERE key = "b" TYPE "knows" NAMESPACE "crm@try""#);
        assert!(b(r#"UNLINK FROM SEMANTIC WHERE key = "a" INTO SEMANTIC WHERE key = "b""#).is_err(), "a link write names its NAMESPACE");
        for q in [r#"PUBLISH TOPIC "orders" (id = 1) NAMESPACE "crm""#, r#"SUBSCRIBE TOPIC "orders" NAMESPACE "crm""#, r#"unsubscribe "sub-1" NAMESPACE "crm""#] {
            assert_eq!(b(q).unwrap(), q);
        }
    }
}

/// Where an ADB server listens: HTTP (`/query`, MCP) and Arrow Flight, FlowR's native
/// wire for `call:adb` (ADB stage G).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbEndpoint {
    pub http: String,
    pub flight: String,
}

/// ADB's default Flight port (`adb serve-http --flight-port`).
pub const ADB_FLIGHT_PORT: u16 = 3001;

/// The endpoint of an ADB at `url`: Flight at `flight_url`, else the same host on
/// ADB's default Flight port.
pub fn adb_endpoint(url: &str, flight_url: Option<&str>) -> AdbEndpoint {
    // C-8: a replica set's nodes, comma separated (only the leader serves Flight; calls
    // find it, see `flight_nodes`). Each HTTP address maps to its Flight address.
    if url.contains(',') && flight_url.is_none() {
        let eps: Vec<AdbEndpoint> = url.split(',').map(str::trim).filter(|u| !u.is_empty()).map(|u| adb_endpoint(u, None)).collect();
        return AdbEndpoint {
            http: eps.iter().map(|e| e.http.as_str()).collect::<Vec<_>>().join(","),
            flight: eps.iter().map(|e| e.flight.as_str()).collect::<Vec<_>>().join(","),
        };
    }
    let http = url.trim_end_matches('/').to_string();
    let flight = match flight_url {
        Some(f) => f.trim_end_matches('/').to_string(),
        None => match reqwest::Url::parse(&http) {
            Ok(mut u) => {
                let _ = u.set_port(Some(ADB_FLIGHT_PORT));
                u.set_path("");
                u.to_string().trim_end_matches('/').to_string()
            }
            Err(_) => http.clone(),
        },
    };
    AdbEndpoint { http, flight }
}

/// The endpoint of an ADB resource (`namespace/name`, or `name` in `default`):
/// `config.url` (or `endpoint`) and optional `config.flight_url`.
pub async fn resolve_adb(storage: &dyn flowr_storage::StorageBackend, resource: &str) -> std::result::Result<AdbEndpoint, String> {
    // Every `call:adb` names its resource: without this, each one read the registry
    // (another ADB round trip) before its own query. An edited resource takes effect
    // within RESOLVED_FOR.
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    const RESOLVED_FOR: std::time::Duration = std::time::Duration::from_secs(5);
    static RESOLVED: OnceLock<Mutex<HashMap<String, (std::time::Instant, AdbEndpoint)>>> = OnceLock::new();
    let cache = RESOLVED.get_or_init(Default::default);
    if let Some((at, ep)) = cache.lock().unwrap().get(resource) {
        if at.elapsed() < RESOLVED_FOR {
            return Ok(ep.clone());
        }
    }
    let ep = resolve_adb_uncached(storage, resource).await?;
    cache.lock().unwrap().insert(resource.to_string(), (std::time::Instant::now(), ep.clone()));
    Ok(ep)
}

async fn resolve_adb_uncached(storage: &dyn flowr_storage::StorageBackend, resource: &str) -> std::result::Result<AdbEndpoint, String> {
    let (ns, name) = resource.split_once('/').unwrap_or(("default", resource));
    let r = storage
        .resources()
        .get(ns, name)
        .await
        .map_err(|e| format!("ADB resource {ns}/{name} not found: {e}"))?
        .in_environment();
    let url = r
        .config
        .get("url")
        .or_else(|| r.config.get("endpoint"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("ADB resource {ns}/{name} has no config.url"))?;
    Ok(adb_endpoint(url, r.config.get("flight_url").and_then(|v| v.as_str())))
}

/// A Flight client for `flight` (channels are cached and reconnect lazily).
/// `authorization` on every Flight call to an ADB that needs a token (W3): the
/// header [`flowr_mcp_core::adb_auth_header`] gives for its HTTP or Flight URL.
#[derive(Clone)]
pub struct AdbBearer(Option<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>);

impl tonic::service::Interceptor for AdbBearer {
    fn call(&mut self, mut req: tonic::Request<()>) -> std::result::Result<tonic::Request<()>, tonic::Status> {
        if let Some(v) = &self.0 {
            req.metadata_mut().insert("authorization", v.clone());
        }
        Ok(req)
    }
}

pub type AdbFlightClient =
    arrow_flight::flight_service_client::FlightServiceClient<tonic::service::interceptor::InterceptedService<tonic::transport::Channel, AdbBearer>>;

/// C-8: which node of an endpoint's replica set calls go to now, per endpoint (shared by
/// every caller), by index into its comma-separated Flight addresses.
static FLIGHT_NODE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, usize>>> = std::sync::OnceLock::new();

/// The Flight addresses of `ep` (one, or a replica set's) and the index calls use now.
fn flight_nodes(ep: &AdbEndpoint) -> (Vec<String>, usize) {
    let nodes: Vec<String> = ep.flight.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let i = FLIGHT_NODE.get_or_init(Default::default).lock().unwrap().get(&ep.flight).copied().unwrap_or(0);
    let n = nodes.len().max(1);
    (nodes, i % n)
}

/// The node at `failed` did not serve the call: the next one is tried, unless another
/// caller already moved on.
fn next_flight_node(ep: &AdbEndpoint, failed: usize) {
    let n = ep.flight.split(',').filter(|s| !s.trim().is_empty()).count();
    if n > 1 {
        let mut m = FLIGHT_NODE.get_or_init(Default::default).lock().unwrap();
        let cur = m.entry(ep.flight.clone()).or_insert(0);
        if *cur % n == failed {
            *cur = (failed + 1) % n;
        }
    }
}

/// Whether an error says this node does not serve the call (a follower, a node down):
/// the call may go to another node of the replica set.
/// Only cases where the request cannot have run (no connection, or a follower's refusal),
/// so sending it again is safe even for an insert: a request that may have reached the
/// leader (a timeout, a reset mid-call) is not re-sent.
fn not_served(err: &str) -> bool {
    err.contains("not the leader") || err.contains("Connection refused") || err.contains("tcp connect") || err.contains("connect error")
}

/// PF-31: whether a call was cut off in transit (the node went down mid-call: a
/// reset, a closed stream, a transport error). It may have run, so it is sent again
/// only when running it twice gives the same result ([`idempotent_action`], reads).
fn cut_off(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    ["transport error", "connection reset", "broken pipe", "stream closed", "h2 protocol error", "connection closed", "no answer", "unexpected eof"]
        .iter()
        .any(|t| e.contains(t))
}

/// Flight actions that may run twice: reads, and writes whose second run changes
/// nothing (an upsert writes the same row again; create-if-missing finds it made).
fn idempotent_action(kind: &str, body: &Value) -> bool {
    match kind {
        "list_tables" | "upsert" => true,
        "aql" => aql_read(body),
        "create_table" => body.get("mode").and_then(|m| m.as_str()).is_none_or(|m| m == "create_if_not_exists" || m == "exist_ok"),
        _ => false,
    }
}

/// Whether a call should go to the next node: not served, or a repeatable call cut off.
fn again(r: &std::result::Result<impl Sized, String>, repeatable: bool) -> bool {
    matches!(r, Err(e) if not_served(e) || (repeatable && cut_off(e)))
}

/// C-16: reads (table and SQL reads, AQL RECALL) go to the replica set's
/// nodes in turn — ADB followers answer them as fresh as the leader — instead of all
/// to the leader. Writes stay with the node calls use now (the leader).
static READ_TURN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The node a read goes to first (`None`: one node, or reads spread off by
/// `FLOWR_ADB_SPREAD_READS=0`).
fn read_node(ep: &AdbEndpoint) -> Option<usize> {
    static SPREAD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*SPREAD.get_or_init(|| std::env::var("FLOWR_ADB_SPREAD_READS").map_or(true, |v| v != "0")) {
        return None;
    }
    let n = ep.flight.split(',').filter(|s| !s.trim().is_empty()).count();
    (n > 1).then(|| READ_TURN.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % n)
}

/// An AQL statement a follower answers: a RECALL (AQL v0.13's only read). A script
/// (`USE …; …`) may write, so it goes to the leader.
fn aql_read(body: &Value) -> bool {
    let q = body.get("query").and_then(|q| q.as_str()).unwrap_or("");
    let first = q.split_whitespace().find(|w| !w.starts_with("--")).unwrap_or("");
    first.eq_ignore_ascii_case("RECALL")
}

/// How long a call waits for a leader when no node serves it (an election, C-21).
const LEADER_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

pub fn flight_client(ep: &AdbEndpoint) -> std::result::Result<AdbFlightClient, String> {
    flight_client_at(ep, None)
}

/// A client for node `at` of `ep` (`None`: the node calls use now).
fn flight_client_at(ep: &AdbEndpoint, at: Option<usize>) -> std::result::Result<AdbFlightClient, String> {
    let (nodes, cur) = flight_nodes(ep);
    let i = at.unwrap_or(cur);
    let flight = nodes.get(i).map(String::as_str).unwrap_or(ep.flight.as_str());
    let http = ep.http.split(',').nth(i).map(str::trim).unwrap_or(ep.http.as_str());
    let bearer = flowr_mcp_core::adb_auth_header(http)
        .or_else(|| flowr_mcp_core::adb_auth_header(flight))
        .and_then(|h| h.parse().ok());
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CHANNELS: OnceLock<Mutex<HashMap<String, tonic::transport::Channel>>> = OnceLock::new();
    let mut cache = CHANNELS.get_or_init(Default::default).lock().unwrap();
    let channel = match cache.get(flight) {
        Some(c) => c.clone(),
        None => {
            let c = tonic::transport::Channel::from_shared(flight.to_string())
                .map_err(|e| format!("bad ADB Flight URL {flight}: {e}"))?
                .connect_timeout(std::time::Duration::from_secs(5))
                .connect_lazy();
            cache.insert(flight.to_string(), c.clone());
            c
        }
    };
    Ok(arrow_flight::flight_service_client::FlightServiceClient::with_interceptor(channel, AdbBearer(bearer))
        .max_decoding_message_size(64 << 20)
        .max_encoding_message_size(64 << 20))
}

/// Read a Flight answer stream to its end (the gRPC status after the last message),
/// briefly. Dropping it after the answer instead resets the stream while ADB's trailers
/// are on the way; once more than a few such resets are recent, h2 counts each late frame
/// as a protocol error, and at 1,024 on a connection it closes that connection, failing
/// every call on it at once (journal writes included).
pub(crate) async fn finish<S: futures::Stream + Unpin>(stream: &mut S) {
    use futures::StreamExt;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), async { while stream.next().await.is_some() {} }).await;
}

/// A Flight `do_action` with a JSON body; the answer's JSON.
/// AO-4: [`flight_action_raw`], counted and timed (`flowr::adb` at debug).
pub async fn flight_action(ep: &AdbEndpoint, kind: &str, body: &Value) -> std::result::Result<Value, String> {
    let t = flowr_metrics::Timer::start();
    // C-8: a node that does not serve the call (a follower, a node down): the next one.
    // C-21: no node serving it (an election in progress): keep trying the nodes in turn,
    // a short pause after each full pass, for up to LEADER_WAIT.
    let (nodes, _) = flight_nodes(ep);
    let deadline = std::time::Instant::now() + LEADER_WAIT;
    let repeatable = idempotent_action(kind, body);
    let first = if kind == "list_tables" || (kind == "aql" && aql_read(body)) { read_node(ep) } else { None };
    let mut r = flight_action_raw(ep, first, kind, body).await;
    if first.is_some() && again(&r, repeatable) {
        r = flight_action_raw(ep, None, kind, body).await;
    }
    let mut tried = 1;
    while again(&r, repeatable) && std::time::Instant::now() < deadline {
        next_flight_node(ep, flight_nodes(ep).1);
        if tried % nodes.len().max(1) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        tried += 1;
        r = flight_action_raw(ep, None, kind, body).await;
    }
    let kind = format!("action:{}", if kind.len() <= 32 && kind.chars().all(|c| c.is_ascii_lowercase() || c == '_') { kind } else { "other" });
    flowr_metrics::observe(&flowr_metrics::get().adb_calls, &flowr_metrics::get().adb_seconds, &[kind.as_str()], r.is_ok(), t.took());
    if t.took() >= flowr_metrics::slow() {
        tracing::info!(target: "flowr::adb", kind = %kind, flight = %ep.flight, ms = t.ms(), ok = r.is_ok(), "slow ADB call");
    } else {
        tracing::debug!(target: "flowr::adb", kind = %kind, flight = %ep.flight, ms = t.ms(), ok = r.is_ok(), "ADB call");
    }
    r
}

async fn flight_action_raw(ep: &AdbEndpoint, at: Option<usize>, kind: &str, body: &Value) -> std::result::Result<Value, String> {
    use futures::StreamExt;
    let mut client = flight_client_at(ep, at)?;
    let action = arrow_flight::Action { r#type: kind.to_string(), body: body.to_string().into() };
    let mut stream = client
        .do_action(action)
        .await
        .map_err(|e| flight_err(ep, e))?
        .into_inner();
    let r = stream
        .next()
        .await
        .ok_or_else(|| format!("ADB {kind}: no answer"))?
        .map_err(|e| flight_err(ep, e))?;
    finish(&mut stream).await;
    serde_json::from_slice(&r.body).map_err(|e| format!("ADB {kind}: answer is not JSON: {e}"))
}

fn flight_err(ep: &AdbEndpoint, e: tonic::Status) -> String {
    match e.code() {
        tonic::Code::Unavailable | tonic::Code::Unknown | tonic::Code::DeadlineExceeded => {
            format!("ADB request to {} failed: {}", ep.flight, e.message())
        }
        // DB-18: a refusal carries its stable code in the `adb-code` metadata.
        _ => with_code(format!("ADB: {}", e.message()), e.metadata().get("adb-code").and_then(|v| v.to_str().ok())),
    }
}

/// `text` with ADB's stable error code (DB-18) as a trailing `[adb:<code>]` tag,
/// which the A3T error handler and the REST edge read with [`adb_code`].
pub fn with_code(text: String, code: Option<&str>) -> String {
    match code.filter(|c| !c.is_empty() && c.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_')) {
        Some(c) => format!("{text} [adb:{c}]"),
        None => text,
    }
}

/// The ADB code tagged on an error by [`with_code`], and the text without the tag.
pub fn adb_code(text: &str) -> (Option<String>, String) {
    if let Some(start) = text.rfind(" [adb:") {
        if let Some(rest) = text[start + 6..].strip_suffix(']') {
            if !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_') {
                return (Some(rest.to_string()), text[..start].to_string());
            }
        }
    }
    (None, text.to_string())
}

/// Rows of an ADB platform table (`do_get`).
/// AO-4: [`flight_get_raw`], counted and timed (`flowr::adb` at debug).
pub async fn flight_get(ep: &AdbEndpoint, table: &str, filter: Option<&str>, limit: usize) -> std::result::Result<Vec<arrow_array::RecordBatch>, String> {
    let t = flowr_metrics::Timer::start();
    // C-8: a node that does not serve the call (a follower, a node down): the next one.
    // C-21: no node serving it (an election in progress): keep trying the nodes in turn,
    // a short pause after each full pass, for up to LEADER_WAIT.
    let (nodes, _) = flight_nodes(ep);
    let deadline = std::time::Instant::now() + LEADER_WAIT;
    let first = read_node(ep);
    let mut r = flight_get_raw(ep, first, table, filter, limit).await;
    if first.is_some() && again(&r, true) {
        r = flight_get_raw(ep, None, table, filter, limit).await;
    }
    let mut tried = 1;
    while again(&r, true) && std::time::Instant::now() < deadline {
        next_flight_node(ep, flight_nodes(ep).1);
        if tried % nodes.len().max(1) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        tried += 1;
        r = flight_get_raw(ep, None, table, filter, limit).await;
    }
    let kind = "get".to_string();
    flowr_metrics::observe(&flowr_metrics::get().adb_calls, &flowr_metrics::get().adb_seconds, &[kind.as_str()], r.is_ok(), t.took());
    if t.took() >= flowr_metrics::slow() {
        tracing::info!(target: "flowr::adb", kind = %kind, flight = %ep.flight, ms = t.ms(), ok = r.is_ok(), "slow ADB call");
    } else {
        tracing::debug!(target: "flowr::adb", kind = %kind, flight = %ep.flight, ms = t.ms(), ok = r.is_ok(), "ADB call");
    }
    r
}

async fn flight_get_raw(ep: &AdbEndpoint, at: Option<usize>, table: &str, filter: Option<&str>, limit: usize) -> std::result::Result<Vec<arrow_array::RecordBatch>, String> {
    use futures::TryStreamExt;
    let mut client = flight_client_at(ep, at)?;
    let ticket = json!({ "table": table, "filter": filter, "limit": limit });
    let stream = client
        .do_get(arrow_flight::Ticket { ticket: ticket.to_string().into() })
        .await
        .map_err(|e| flight_err(ep, e))?
        .into_inner()
        .map_err(|e| arrow_flight::error::FlightError::Tonic(Box::new(e)));
    arrow_flight::decode::FlightRecordBatchStream::new_from_flight_data(stream)
        .try_collect()
        .await
        .map_err(|e| format!("ADB query {table}: {e}"))
}

/// Read-only SQL over an ADB's memory and platform tables (`do_get` with a `sql`
/// ticket; the superadmin's console, not flows).
/// AO-4: [`flight_sql_raw`], counted and timed (`flowr::adb` at debug).
pub async fn flight_sql(ep: &AdbEndpoint, sql: &str, limit: Option<u64>) -> std::result::Result<Vec<arrow_array::RecordBatch>, String> {
    let t = flowr_metrics::Timer::start();
    // C-8: a node that does not serve the call (a follower, a node down): the next one.
    // C-21: no node serving it (an election in progress): keep trying the nodes in turn,
    // a short pause after each full pass, for up to LEADER_WAIT.
    let (nodes, _) = flight_nodes(ep);
    let deadline = std::time::Instant::now() + LEADER_WAIT;
    let first = read_node(ep);
    let mut r = flight_sql_raw(ep, first, sql, limit).await;
    if first.is_some() && again(&r, true) {
        r = flight_sql_raw(ep, None, sql, limit).await;
    }
    let mut tried = 1;
    while again(&r, true) && std::time::Instant::now() < deadline {
        next_flight_node(ep, flight_nodes(ep).1);
        if tried % nodes.len().max(1) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        tried += 1;
        r = flight_sql_raw(ep, None, sql, limit).await;
    }
    let kind = "sql".to_string();
    flowr_metrics::observe(&flowr_metrics::get().adb_calls, &flowr_metrics::get().adb_seconds, &[kind.as_str()], r.is_ok(), t.took());
    if t.took() >= flowr_metrics::slow() {
        tracing::info!(target: "flowr::adb", kind = %kind, flight = %ep.flight, ms = t.ms(), ok = r.is_ok(), "slow ADB call");
    } else {
        tracing::debug!(target: "flowr::adb", kind = %kind, flight = %ep.flight, ms = t.ms(), ok = r.is_ok(), "ADB call");
    }
    r
}

async fn flight_sql_raw(ep: &AdbEndpoint, at: Option<usize>, sql: &str, limit: Option<u64>) -> std::result::Result<Vec<arrow_array::RecordBatch>, String> {
    use futures::TryStreamExt;
    let mut client = flight_client_at(ep, at)?;
    let ticket = json!({ "sql": sql, "limit": limit });
    let stream = client
        .do_get(arrow_flight::Ticket { ticket: ticket.to_string().into() })
        .await
        .map_err(|e| flight_err(ep, e))?
        .into_inner()
        .map_err(|e| arrow_flight::error::FlightError::Tonic(Box::new(e)));
    arrow_flight::decode::FlightRecordBatchStream::new_from_flight_data(stream)
        .try_collect()
        .await
        .map_err(|e| format!("ADB SQL: {e}"))
}

/// Insert a batch into an ADB platform table (`do_put`) — or, with `replace`, make
/// it the whole table (columns included); the answer's metadata.
/// AO-4: [`flight_put_raw`], counted and timed (`flowr::adb` at debug).
pub async fn flight_put(ep: &AdbEndpoint, table: &str, batch: arrow_array::RecordBatch, replace: bool) -> std::result::Result<Value, String> {
    let t = flowr_metrics::Timer::start();
    // C-8: a node that does not serve the call: the next one (the batch's arrays are
    // shared, so the copy is cheap).
    // C-21: no node serving it (an election in progress): keep trying the nodes in turn,
    // a short pause after each full pass, for up to LEADER_WAIT.
    let (nodes, _) = flight_nodes(ep);
    let deadline = std::time::Instant::now() + LEADER_WAIT;
    let mut r = flight_put_raw(ep, table, batch.clone(), replace).await;
    let mut tried = 1;
    while matches!(&r, Err(e) if not_served(e)) && std::time::Instant::now() < deadline {
        next_flight_node(ep, flight_nodes(ep).1);
        if tried % nodes.len().max(1) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        tried += 1;
        r = flight_put_raw(ep, table, batch.clone(), replace).await;
    }
    let kind = "put".to_string();
    flowr_metrics::observe(&flowr_metrics::get().adb_calls, &flowr_metrics::get().adb_seconds, &[kind.as_str()], r.is_ok(), t.took());
    if t.took() >= flowr_metrics::slow() {
        tracing::info!(target: "flowr::adb", kind = %kind, flight = %ep.flight, ms = t.ms(), ok = r.is_ok(), "slow ADB call");
    } else {
        tracing::debug!(target: "flowr::adb", kind = %kind, flight = %ep.flight, ms = t.ms(), ok = r.is_ok(), "ADB call");
    }
    r
}

async fn flight_put_raw(ep: &AdbEndpoint, table: &str, batch: arrow_array::RecordBatch, replace: bool) -> std::result::Result<Value, String> {
    use futures::StreamExt;
    let mut client = flight_client(ep)?;
    let mut path = vec![table.to_string()];
    if replace {
        path.push("replace".into());
    }
    let data = arrow_flight::encode::FlightDataEncoderBuilder::new()
        .with_flight_descriptor(Some(arrow_flight::FlightDescriptor::new_path(path)))
        .build(futures::stream::iter(vec![Ok(batch)]))
        .filter_map(|r| async move { r.ok() });
    let mut results = client.do_put(data).await.map_err(|e| flight_err(ep, e))?.into_inner();
    let r = results
        .next()
        .await
        .ok_or_else(|| format!("ADB insert {table}: no answer"))?
        .map_err(|e| flight_err(ep, e))?;
    finish(&mut results).await;
    serde_json::from_slice(&r.app_metadata).map_err(|e| format!("ADB insert {table}: {e}"))
}

/// Run one AQL statement on ADB over Flight (`do_action("aql")`); returns the query
/// result (`{success, data: {type, …}, metadata}`), as `POST /query` did.
pub async fn adb_query(ep: &AdbEndpoint, query: &str, variables: Option<&Value>) -> std::result::Result<Value, String> {
    let mut body = json!({ "query": query });
    if let Some(v) = variables.filter(|v| !v.is_null()) {
        body["variables"] = v.clone();
    }
    let answer = flight_action(ep, "aql", &body).await?;
    if answer.get("success").and_then(|s| s.as_bool()) != Some(true) {
        let err = answer.get("error").and_then(|e| e.as_str()).unwrap_or("unknown error");
        return Err(with_code(format!("ADB: {err}"), answer.get("code").and_then(|c| c.as_str())));
    }
    Ok(answer.get("data").cloned().unwrap_or(Value::Null))
}

/// Flatten a query result into table rows for consoles: records become
/// `{id, memory_type, …data}`; other results (stored, affected, …) are returned as is.
pub fn result_rows(result: &Value) -> Value {
    let data = result.get("data").unwrap_or(result);
    match data.get("records").and_then(|r| r.as_array()) {
        Some(records) => Value::Array(
            records
                .iter()
                .map(|rec| {
                    let mut row = serde_json::Map::new();
                    row.insert("id".into(), rec.get("id").cloned().unwrap_or(Value::Null));
                    row.insert("memory_type".into(), rec.get("memory_type").cloned().unwrap_or(Value::Null));
                    if let Some(fields) = rec.get("data").and_then(|d| d.as_object()) {
                        for (k, v) in fields {
                            row.insert(k.clone(), v.clone());
                        }
                    }
                    Value::Object(row)
                })
                .collect(),
        ),
        None => data.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flight_endpoint_defaults_to_the_same_host() {
        assert_eq!(adb_endpoint("http://adb.a3t.svc.cluster.local:3000", None).flight, "http://adb.a3t.svc.cluster.local:3001");
        assert_eq!(adb_endpoint("http://adb:3000/", None).http, "http://adb:3000");
        assert_eq!(adb_endpoint("http://adb", None).flight, "http://adb:3001");
        assert_eq!(adb_endpoint("http://x:1", Some("http://y:9/")).flight, "http://y:9");
    }

    #[test]
    fn rows_flatten_records() {
        let r = json!({"success": true, "data": {"type": "records", "count": 1, "records": [
            {"id": "a", "memory_type": "WORKING", "data": {"name": "alice", "score": 95}, "metadata": {}}
        ]}});
        assert_eq!(result_rows(&r), json!([{"id": "a", "memory_type": "WORKING", "name": "alice", "score": 95}]));
    }

    #[test]
    fn codes_ride_along_and_come_back_off() {
        let t = with_code("ADB: namespace \"a\" already has a branch \"x\"".into(), Some("branch_exists"));
        assert_eq!(t, "ADB: namespace \"a\" already has a branch \"x\" [adb:branch_exists]");
        let (code, msg) = adb_code(&t);
        assert_eq!(code.as_deref(), Some("branch_exists"));
        assert_eq!(msg, "ADB: namespace \"a\" already has a branch \"x\"");
        assert_eq!(adb_code("ADB: plain").0, None);
        // Nothing odd is ever tagged.
        assert_eq!(with_code("x".into(), Some("Bad Code]")), "x");
    }

    #[test]
    fn flight_refusals_carry_their_code() {
        let ep = adb_endpoint("http://adb:8080", None);
        let mut st = tonic::Status::already_exists("tag \"v1\" exists");
        st.metadata_mut().insert("adb-code", "tag_exists".parse().unwrap());
        assert_eq!(flight_err(&ep, st), "ADB: tag \"v1\" exists [adb:tag_exists]");
        let st = tonic::Status::unavailable("connection refused");
        assert!(flight_err(&ep, st).starts_with("ADB request to"));
    }

    #[test]
    fn rows_pass_through_non_records() {
        let r = json!({"success": true, "data": {"type": "affected", "count": 2}});
        assert_eq!(result_rows(&r), json!({"type": "affected", "count": 2}));
    }
}
