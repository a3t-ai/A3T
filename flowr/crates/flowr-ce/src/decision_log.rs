//! The durable decision log (AUD-1): every policy decision, from every pod, kept
//! in ADB EPISODIC memory — one namespace per org, `_audit.<org id>` (and
//! `_audit.platform` for platform-wide calls and personal projects).
//!
//! - A project's namespace is `[a-z0-9-]` and cannot start with `_`, so no project
//!   flow or agent can name these namespaces (`call:adb` lets a project name only
//!   its own).
//! - Each decision is one `STORE INTO EPISODIC` with `key = <decision id>`, so a
//!   retried write never duplicates. Its `content` is a sentence, so `RECALL FROM
//!   EPISODIC LIKE "…"` finds decisions by meaning as well as by field.
//! - Pods queue decisions and write them about once a second; a write that fails
//!   is retried with the next batch. Only a queue over [`QUEUE`] drops (and the
//!   count of dropped decisions is logged).
//! - Reading (`GET /security/audit`) is `RECALL FROM EPISODIC WHERE … ORDER BY
//!   ts_ms DESC LIMIT n NAMESPACE (…)`; the answer carries the AQL it ran.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use flowr_policy::audit::AuditRecord;
use flowr_storage::StorageBackend;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::handlers::agent_memory::aql_str;
use crate::handlers::call_adb::{adb_query, resolve_adb, AdbEndpoint};

/// Decisions waiting to be written, at most.
const QUEUE: usize = 200_000;
const FLUSH: Duration = Duration::from_secs(1);
const BATCH: usize = 2_000;
const PARALLEL: usize = 8;
/// How often each process writes its chains' heads as an anchor (AUD-4).
const ANCHOR_EVERY: Duration = Duration::from_secs(3600);
pub const ANCHORS: &str = "audit_anchors";
/// How long an agent's running version is remembered.
const VERSION_TTL: Duration = Duration::from_secs(30);

static DROPPED: AtomicU64 = AtomicU64::new(0);
static STORAGE: OnceLock<Arc<dyn StorageBackend>> = OnceLock::new();

/// The namespace holding an org's decisions.
pub fn namespace(org: &str) -> String {
    format!("_audit.{org}")
}

/// Send every decision this pod makes to the durable log.
pub fn install(storage: Arc<dyn StorageBackend>) {
    if STORAGE.set(storage.clone()).is_err() {
        return;
    }
    let (tx, rx) = mpsc::channel::<AuditRecord>(QUEUE);
    flowr_policy::audit::set_sink(Box::new(move |rec| {
        if tx.try_send(rec).is_err() {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }));
    tokio::spawn(writer(storage, rx));
    info!(resource = %crate::agent_space::org_adb(""), "decision log: durable (ADB EPISODIC, _audit.<org>)");
}

/// One decision as an AQL `STORE` into its org's namespace.
pub fn store_statement(r: &AuditRecord) -> String {
    let mut f: Vec<String> = vec![
        format!("key = {}", aql_str(&r.id)),
        "kind = \"decision\"".into(),
        format!("ts_ms = {}", r.ts_ms),
        format!("at = {}", aql_str(&chrono::DateTime::from_timestamp_millis(r.ts_ms as i64).map(|t| t.to_rfc3339()).unwrap_or_default())),
        format!("pod = {}", aql_str(&r.pod)),
        format!("principal_kind = {}", aql_str(&r.principal_kind)),
        format!("principal_id = {}", aql_str(&r.principal_id)),
        format!("action = {}", aql_str(&r.action)),
        format!("resource_type = {}", aql_str(&r.resource_type)),
        format!("resource_id = {}", aql_str(&r.resource_id)),
        format!("decision = {}", aql_str(&r.decision)),
        format!("content = {}", aql_str(&sentence(r))),
    ];
    let mut opt = |k: &str, v: &Option<String>| {
        if let Some(v) = v.as_deref().filter(|v| !v.is_empty()) {
            f.push(format!("{k} = {}", aql_str(v)));
        }
    };
    opt("on_behalf_of", &r.on_behalf_of);
    opt("impersonated_by", &r.impersonated_by);
    opt("reason", &r.reason);
    opt("project", &r.project);
    opt("org", &r.org);
    opt("agent", &r.agent);
    opt("agent_version", &r.agent_version);
    opt("agent_owner", &r.agent_owner);
    opt("session", &r.session);
    opt("flow", &r.flow);
    opt("policy_version", &r.policy_version);
    opt("chain", &r.chain);
    opt("prev_hash", &r.prev_hash);
    opt("hash", &r.hash);
    if !r.policy_ids.is_empty() {
        f.push(format!("policy_ids = {}", aql_str(&r.policy_ids.join(","))));
    }
    if !r.roles.is_empty() {
        f.push(format!("roles = {}", aql_str(&r.roles.join(","))));
    }
    if let Some(n) = r.seq {
        f.push(format!("seq = {n}"));
    }
    let org = r.org.as_deref().unwrap_or("platform");
    format!("STORE INTO EPISODIC ({}) NAMESPACE {}", f.join(", "), aql_str(&namespace(org)))
}

/// The decision in a sentence, for recall by meaning.
fn sentence(r: &AuditRecord) -> String {
    let who = match (&r.agent, &r.on_behalf_of) {
        (Some(a), Some(p)) => format!("agent {a} for {p}"),
        (Some(a), None) => format!("agent {a}"),
        _ => format!("{} {}", r.principal_kind, r.principal_id),
    };
    let by = r.impersonated_by.as_deref().map(|s| format!(" (SuperAdmin {s} acting as them)")).unwrap_or_default();
    let place = r.project.as_deref().map(|p| format!(" in project {p}")).unwrap_or_default();
    let why = r.reason.as_deref().map(|w| format!(": {w}")).unwrap_or_default();
    format!("{} {who}{by} {} {} {}{place}{why}", r.decision, r.action, r.resource_type, r.resource_id)
}

/// The record a `RECALL` row holds (its `data`).
pub fn from_row(row: &Value) -> Value {
    let d = row.get("data").unwrap_or(row);
    let s = |k: &str| d.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let list = |k: &str| s(k).map(|v| v.split(',').filter(|x| !x.is_empty()).map(str::to_string).collect::<Vec<_>>()).unwrap_or_default();
    json!({
        "id": s("key").or_else(|| row.get("key").and_then(|k| k.as_str()).map(str::to_string)),
        "ts_ms": d.get("ts_ms").and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64))).unwrap_or(0),
        "pod": s("pod"), "principal_kind": s("principal_kind"), "principal_id": s("principal_id"),
        "on_behalf_of": s("on_behalf_of"), "impersonated_by": s("impersonated_by"),
        "action": s("action"), "resource_type": s("resource_type"), "resource_id": s("resource_id"),
        "decision": s("decision"), "policy_ids": list("policy_ids"), "reason": s("reason"),
        "project": s("project"), "org": s("org"), "roles": list("roles"),
        "agent": s("agent"), "agent_version": s("agent_version"), "agent_owner": s("agent_owner"),
        "session": s("session"), "flow": s("flow"), "policy_version": s("policy_version"),
        "chain": s("chain"), "seq": d.get("seq").and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64))),
        "prev_hash": s("prev_hash"), "hash": s("hash"),
    })
}

/// AUD-4: what a record's hash covers, in one string — the same from a record
/// about to be written and from one read back.
pub fn canonical(v: &Value) -> String {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let list = |k: &str| v.get(k).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(",")).unwrap_or_default();
    let n = |k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    [s("id"), n("ts_ms").to_string(), s("principal_kind"), s("principal_id"), s("on_behalf_of"), s("impersonated_by"),
     s("action"), s("resource_type"), s("resource_id"), s("decision"), s("reason"), s("project"), s("org"),
     list("policy_ids"), list("roles"), s("agent"), s("policy_version"), s("chain"), n("seq").to_string(), s("prev_hash")]
        .join("\u{1f}")
}

pub fn hash_of(v: &Value) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(canonical(v).as_bytes()))
}

/// A record as `from_row` would read it back (to hash it the same way).
fn as_read(r: &AuditRecord) -> Value {
    json!({
        "id": r.id, "ts_ms": r.ts_ms, "principal_kind": r.principal_kind, "principal_id": r.principal_id,
        "on_behalf_of": r.on_behalf_of, "impersonated_by": r.impersonated_by, "action": r.action,
        "resource_type": r.resource_type, "resource_id": r.resource_id, "decision": r.decision, "reason": r.reason,
        "project": r.project, "org": r.org, "policy_ids": r.policy_ids, "roles": r.roles, "agent": r.agent,
        "policy_version": r.policy_version, "chain": r.chain, "seq": r.seq, "prev_hash": r.prev_hash,
    })
}

struct Writer {
    storage: Arc<dyn StorageBackend>,
    endpoint: Option<(AdbEndpoint, Instant)>,
    versions: HashMap<String, (String, Instant)>,
    /// AUD-4: per org, this process's chain: (last seq, last hash).
    chains: HashMap<String, (u64, String)>,
    /// This process's chain name: `<pod>@<start ms>`.
    me: String,
}

impl Writer {
    /// Put `r` on its org's chain in this process (once: a retry keeps it).
    fn chain(&mut self, r: &mut AuditRecord) {
        link(&mut self.chains, &self.me, r);
    }

    /// The heads of this process's chains, for an anchor.
    fn heads(&self) -> Vec<(String, u64, String)> {
        self.chains.iter().map(|(org, (n, h))| (format!("{}/{org}", self.me), *n, h.clone())).collect()
    }
}

impl Writer {
    async fn endpoint(&mut self) -> Result<AdbEndpoint, String> {
        if let Some((ep, at)) = &self.endpoint {
            if at.elapsed() < Duration::from_secs(60) {
                return Ok(ep.clone());
            }
        }
        let ep = resolve_adb(self.storage.as_ref(), &crate::agent_space::org_adb("")).await?;
        self.endpoint = Some((ep.clone(), Instant::now()));
        Ok(ep)
    }

    async fn agent_version(&mut self, agent: &str) -> Option<String> {
        if let Some((v, at)) = self.versions.get(agent) {
            if at.elapsed() < VERSION_TTL {
                return Some(v.clone()).filter(|v| !v.is_empty());
            }
        }
        let v = crate::agent_space::running_version(self.storage.as_ref(), agent).await;
        if self.versions.len() > 10_000 {
            self.versions.clear();
        }
        self.versions.insert(agent.to_string(), (v.clone(), Instant::now()));
        Some(v).filter(|v| !v.is_empty())
    }

    /// Org and agent version: what the deciding code didn't know.
    async fn complete(&mut self, r: &mut AuditRecord) {
        if r.org.is_none() {
            r.org = Some(crate::access::audit_org(r.project.as_deref()).await);
        }
        // Events name an org by its handle or its id; the log is kept by id.
        if let Some(o) = r.org.clone().filter(|o| o != "platform" && !o.starts_with("org-")) {
            if let Some(storage) = STORAGE.get() {
                if let Some(id) = crate::org_policy::org_id(storage.as_ref(), &o).await {
                    r.org = Some(id);
                }
            }
        }
        if r.agent_version.is_none() {
            if let Some(a) = r.agent.clone() {
                r.agent_version = self.agent_version(&a).await;
            }
        }
    }
}

async fn writer(storage: Arc<dyn StorageBackend>, mut rx: mpsc::Receiver<AuditRecord>) {
    let pod = std::env::var("HOSTNAME").unwrap_or_else(|_| "pod".into());
    let mut w = Writer { storage, endpoint: None, versions: HashMap::new(), chains: HashMap::new(),
        me: format!("{pod}@{}", chrono::Utc::now().timestamp_millis()) };
    let mut last_anchor = Instant::now();
    let mut pending: Vec<AuditRecord> = Vec::new();
    let mut last_warn = Instant::now() - Duration::from_secs(3600);
    loop {
        // Wait for the first decision (or retry what is pending), then gather
        // whatever else arrives within FLUSH.
        if pending.is_empty() {
            match rx.recv().await {
                Some(r) => pending.push(r),
                None => return,
            }
        }
        let until = tokio::time::Instant::now() + FLUSH;
        while pending.len() < BATCH {
            match tokio::time::timeout_at(until, rx.recv()).await {
                Ok(Some(r)) => pending.push(r),
                Ok(None) => break,
                Err(_) => break,
            }
        }
        let dropped = DROPPED.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            warn!(dropped, "decision log: queue full; decisions dropped");
        }
        let ep = match w.endpoint().await {
            Ok(ep) => ep,
            Err(e) => {
                if last_warn.elapsed() > Duration::from_secs(60) {
                    warn!(error = %e, pending = pending.len(), "decision log: ADB unavailable; keeping decisions to retry");
                    last_warn = Instant::now();
                }
                if pending.len() > QUEUE {
                    let over = pending.len() - QUEUE;
                    pending.drain(..over);
                    DROPPED.fetch_add(over as u64, Ordering::Relaxed);
                }
                tokio::time::sleep(FLUSH).await;
                continue;
            }
        };
        let mut batch = std::mem::take(&mut pending);
        for r in batch.iter_mut() {
            w.complete(r).await;
            // RT-10: the log keeps no personal data or secrets in its free text.
            if let Some(reason) = r.reason.as_mut() {
                *reason = crate::pii::clean(reason);
            }
            w.chain(r);
        }
        // AUD-4: the heads of this process's chains, kept apart every hour.
        if last_anchor.elapsed() > ANCHOR_EVERY {
            anchor(w.storage.as_ref(), &w.heads()).await;
            last_anchor = Instant::now();
        }
        let results: Vec<(AuditRecord, Result<Value, String>)> = futures::stream::iter(batch.into_iter().map(|r| {
            let ep = ep.clone();
            async move {
                let res = adb_query(&ep, &store_statement(&r), None).await;
                (r, res)
            }
        }))
        .buffer_unordered(PARALLEL)
        .collect()
        .await;
        let mut failed = 0usize;
        let mut written = Vec::new();
        for (r, res) in results {
            match res {
                Ok(_) => written.push(r),
                Err(e) => {
                    failed += 1;
                    if last_warn.elapsed() > Duration::from_secs(60) {
                        warn!(error = %e, "decision log: a write failed; it is retried");
                        last_warn = Instant::now();
                    }
                    pending.push(r);
                }
            }
        }
        // AUD-7: what was written goes on to the SIEM sinks, beside the log.
        crate::audit_export::send(written);
        if failed > 0 {
            tokio::time::sleep(FLUSH).await;
        }
    }
}

/// What a read asks for (`GET /security/audit`).
#[derive(Debug, Default, Clone)]
pub struct Query {
    pub orgs: Vec<String>,
    pub project: Option<String>,
    pub agent: Option<String>,
    pub agent_owner: Option<String>,
    pub principal: Option<String>,
    /// The person an agent acted for.
    pub on_behalf_of: Option<String>,
    pub action: Option<String>,
    pub decision: Option<String>,
    pub since_ms: Option<u64>,
    pub before_ms: Option<u64>,
    /// Ranked by meaning (`LIKE`), newest first otherwise.
    pub like: Option<String>,
    pub limit: usize,
}

/// The AQL a read runs, and its variables.
pub fn recall_statement(q: &Query) -> (String, Option<Value>) {
    let mut w: Vec<String> = vec!["kind = \"decision\"".into()];
    let mut eq = |k: &str, v: &Option<String>| {
        if let Some(v) = v.as_deref().filter(|v| !v.is_empty()) {
            w.push(format!("{k} = {}", aql_str(v)));
        }
    };
    eq("project", &q.project);
    eq("agent", &q.agent);
    eq("agent_owner", &q.agent_owner);
    eq("principal_id", &q.principal);
    eq("on_behalf_of", &q.on_behalf_of);
    eq("action", &q.action);
    eq("decision", &q.decision);
    if let Some(t) = q.since_ms {
        w.push(format!("ts_ms >= {t}"));
    }
    if let Some(t) = q.before_ms {
        w.push(format!("ts_ms < {t}"));
    }
    let ns = q.orgs.iter().map(|o| aql_str(&namespace(o))).collect::<Vec<_>>();
    let ns = if ns.len() == 1 { ns[0].clone() } else { format!("({})", ns.join(", ")) };
    let limit = q.limit.clamp(1, 500);
    match q.like.as_deref().filter(|l| !l.trim().is_empty()) {
        // AQL takes WHERE or LIKE, not both: a search by meaning is ranked by ADB
        // and filtered here (`matches`).
        Some(l) => (format!("RECALL FROM EPISODIC LIKE $q LIMIT {limit} NAMESPACE {ns}"), Some(json!({ "q": l }))),
        None => (format!("RECALL FROM EPISODIC WHERE {} ORDER BY ts_ms DESC LIMIT {limit} NAMESPACE {ns}", w.join(" AND ")), None),
    }
}

/// Whether a decision passes the read's filters (for a ranked read).
pub fn matches(q: &Query, v: &Value) -> bool {
    let is = |k: &str, want: &Option<String>| want.as_deref().filter(|w| !w.is_empty()).is_none_or(|w| v.get(k).and_then(|x| x.as_str()) == Some(w));
    let ts = v.get("ts_ms").and_then(|t| t.as_u64()).unwrap_or(0);
    is("project", &q.project) && is("agent", &q.agent) && is("agent_owner", &q.agent_owner)
        && is("principal_id", &q.principal) && is("on_behalf_of", &q.on_behalf_of) && is("action", &q.action) && is("decision", &q.decision)
        && q.since_ms.is_none_or(|t| ts >= t) && q.before_ms.is_none_or(|t| ts < t)
}

/// Run a read against the durable log.
pub async fn recall(q: &Query) -> Result<(Vec<Value>, String), String> {
    let storage = STORAGE.get().ok_or("the durable decision log is not installed on this server")?;
    if q.orgs.is_empty() {
        return Ok((Vec::new(), String::new()));
    }
    let ep = resolve_adb(storage.as_ref(), &crate::agent_space::org_adb("")).await?;
    let (aql, vars) = recall_statement(q);
    let res = adb_query(&ep, &aql, vars.as_ref()).await?;
    let data = res.get("data").unwrap_or(&res);
    let rows = data.get("records").and_then(|r| r.as_array()).cloned().unwrap_or_default();
    let mut out: Vec<Value> = rows.iter().filter(|r| r.get("data").unwrap_or(r).get("kind").and_then(|k| k.as_str()) == Some("decision")).map(from_row).collect();
    if q.like.is_some() {
        out.retain(|v| matches(q, v));
    } else {
        out.sort_by(|a, b| b["ts_ms"].as_u64().cmp(&a["ts_ms"].as_u64()));
    }
    Ok((out, aql))
}

/// Put `r` on its org's chain `me/<org>` (once: a retry keeps it).
fn link(chains: &mut HashMap<String, (u64, String)>, me: &str, r: &mut AuditRecord) {
    if r.hash.is_some() {
        return;
    }
    let org = r.org.clone().unwrap_or_else(|| "platform".into());
    let (seq, prev) = chains.get(&org).cloned().unwrap_or((0, String::new()));
    r.chain = Some(format!("{me}/{org}"));
    r.seq = Some(seq + 1);
    r.prev_hash = Some(prev);
    let h = hash_of(&as_read(r));
    r.hash = Some(h.clone());
    chains.insert(org, (seq + 1, h));
}

/// AUD-4: the heads of a process's chains, kept outside the log (a platform
/// table; an object-locked bucket when one is configured) so an edit or a
/// removal inside the log can't hide.
async fn anchor(s: &dyn StorageBackend, heads: &[(String, u64, String)]) {
    if heads.is_empty() {
        return;
    }
    let db = crate::platform_db::open_platform_db(s, Some(crate::handlers::call_lancedb::PLATFORM_DB_RESOURCE), crate::handlers::call_lancedb::DEFAULT_PLATFORM_DB_PATH).await;
    let schema: serde_json::Map<String, Value> = ["chain", "seq", "hash", "at"].iter().map(|c| (c.to_string(), json!("string"))).collect();
    if let Err(e) = db.create_table(ANCHORS, Some(&schema), "create_if_not_exists").await {
        warn!(error = %e, "decision log: anchors table unavailable");
        return;
    }
    let at = chrono::Utc::now().to_rfc3339();
    let rows: Vec<Value> = heads.iter().map(|(c, n, h)| json!({ "chain": c, "seq": n.to_string(), "hash": h, "at": at })).collect();
    if let Err(e) = db.insert(ANCHORS, &rows).await {
        warn!(error = %e, "decision log: anchor not written");
    }
}

/// AUD-4: check an org's chains. Every record read back is hashed again and
/// linked to the one before it; a missing sequence number is a gap, a hash
/// that doesn't match is an edit, and each anchor must match its record.
pub async fn verify(org: &str, since_ms: u64, max: usize) -> Result<Value, String> {
    let mut q = Query { orgs: vec![org.to_string()], since_ms: Some(since_ms), limit: 500, ..Default::default() };
    let mut rows: Vec<Value> = Vec::new();
    loop {
        let (page, _) = recall(&q).await?;
        let last = page.last().and_then(|r| r["ts_ms"].as_u64());
        let n = page.len();
        rows.extend(page);
        if n < 500 || rows.len() >= max || last.is_none() {
            break;
        }
        q.before_ms = last;
    }
    let mut chains: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
    let mut unchained = 0usize;
    for r in rows.iter() {
        match r["chain"].as_str() {
            Some(c) if r["seq"].as_u64().is_some() => chains.entry(c.to_string()).or_default().push(r.clone()),
            _ => unchained += 1,
        }
    }
    let (mut gaps, mut edits, mut broken_links) = (Vec::new(), Vec::new(), Vec::new());
    let mut heads: HashMap<String, (u64, String)> = HashMap::new();
    for (c, list) in chains.iter_mut() {
        list.sort_by_key(|r| r["seq"].as_u64().unwrap_or(0));
        let mut prev: Option<&Value> = None;
        for r in list.iter() {
            let seq = r["seq"].as_u64().unwrap_or(0);
            if r["hash"].as_str() != Some(hash_of(r).as_str()) {
                edits.push(json!({ "chain": c, "seq": seq, "id": r["id"] }));
            }
            if let Some(p) = prev {
                let pseq = p["seq"].as_u64().unwrap_or(0);
                if seq != pseq + 1 {
                    gaps.push(json!({ "chain": c, "after": pseq, "before": seq, "missing": seq.saturating_sub(pseq + 1) }));
                } else if r["prev_hash"] != p["hash"] {
                    broken_links.push(json!({ "chain": c, "seq": seq }));
                }
            }
            prev = Some(r);
        }
        if let Some(l) = list.last() {
            heads.insert(c.clone(), (l["seq"].as_u64().unwrap_or(0), l["hash"].as_str().unwrap_or_default().to_string()));
        }
    }
    // Anchors: each one for a chain read here must match the record at its seq.
    let mut anchors_checked = 0usize;
    let mut anchor_mismatch = Vec::new();
    if let Some(storage) = STORAGE.get() {
        let all = crate::handlers::call_lancedb::query_platform_table(storage.as_ref(), ANCHORS, None, 100_000).await.unwrap_or_default();
        for a in all {
            let c = a.get("chain").and_then(|v| v.as_str()).unwrap_or_default();
            let Some(list) = chains.get(c) else { continue };
            let seq: u64 = a.get("seq").and_then(|v| v.as_str()).and_then(|v| v.parse().ok()).unwrap_or(0);
            let want = a.get("hash").and_then(|v| v.as_str()).unwrap_or_default();
            let first = list.first().and_then(|r| r["seq"].as_u64()).unwrap_or(0);
            match list.iter().find(|r| r["seq"].as_u64() == Some(seq)) {
                Some(r) => {
                    anchors_checked += 1;
                    if r["hash"].as_str() != Some(want) { anchor_mismatch.push(json!({ "chain": c, "seq": seq })); }
                }
                // Before what was read (older, or forgotten by retention): not checkable here.
                None if seq < first => {}
                None => anchor_mismatch.push(json!({ "chain": c, "seq": seq, "missing": true })),
            }
        }
    }
    let ok = gaps.is_empty() && edits.is_empty() && broken_links.is_empty() && anchor_mismatch.is_empty();
    Ok(json!({
        "org": org, "ok": ok, "records": rows.len(), "chains": chains.len(), "unchained": unchained,
        "gaps": gaps, "edits": edits, "broken_links": broken_links,
        "anchors_checked": anchors_checked, "anchor_mismatch": anchor_mismatch,
        "note": "Records written before chaining began are counted as unchained. A chain's first record read here starts its check: anything older was forgotten by retention (logged as audit:purge) or is outside the window.",
    }))
}

/// Run one statement against the log's ADB (AUD-6 retention): the count it
/// reports, when it reports one.
pub async fn run(aql: &str) -> Result<u64, String> {
    let storage = STORAGE.get().ok_or("the durable decision log is not installed on this server")?;
    let ep = resolve_adb(storage.as_ref(), &crate::agent_space::org_adb("")).await?;
    let res = adb_query(&ep, aql, None).await?;
    let data = res.get("data").unwrap_or(&res);
    Ok(data.get("count").and_then(|c| c.as_u64()).unwrap_or(0))
}

/// Whether the durable log is on in this process.
pub fn installed() -> bool {
    STORAGE.get().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> AuditRecord {
        AuditRecord {
            id: "pod-1".into(), ts_ms: 1_795_000_000_000, pod: "pod".into(),
            principal_kind: "agent".into(), principal_id: "agent:agt-1:s".into(),
            on_behalf_of: Some("u-1".into()), action: "tool:call".into(),
            resource_type: "Tool".into(), resource_id: "delete_flow".into(),
            decision: "deny".into(), policy_ids: vec!["org/acme/no-delete".into()],
            reason: Some("no \"deletes\" here".into()), project: Some("acme-web".into()),
            org: Some("org-acme".into()), roles: vec!["Developer".into()],
            agent: Some("agt-1".into()), agent_version: Some("3".into()),
            policy_version: Some("abcd".into()), ..Default::default()
        }
    }

    #[test]
    fn a_decision_is_stored_in_its_orgs_namespace_by_its_id() {
        let s = store_statement(&rec());
        assert!(s.starts_with("STORE INTO EPISODIC (key = \"pod-1\", kind = \"decision\", ts_ms = 1795000000000"));
        assert!(s.ends_with("NAMESPACE \"_audit.org-acme\""));
        assert!(s.contains("reason = \"no \\\"deletes\\\" here\""));
        assert!(s.contains("agent = \"agt-1\"") && s.contains("agent_version = \"3\"") && s.contains("policy_version = \"abcd\""));
        assert_eq!(crate::handlers::call_adb::named_namespaces(&s), vec!["_audit.org-acme"]);
        // No project may name it.
        assert!(crate::handlers::call_adb::project_namespaces_only(&s, "acme-web").is_err());
    }

    #[test]
    fn a_read_names_its_filters_and_namespaces() {
        let q = Query { orgs: vec!["org-a".into(), "platform".into()], agent: Some("agt-1".into()), before_ms: Some(10), limit: 50, ..Default::default() };
        let (aql, vars) = recall_statement(&q);
        assert_eq!(aql, "RECALL FROM EPISODIC WHERE kind = \"decision\" AND agent = \"agt-1\" AND ts_ms < 10 ORDER BY ts_ms DESC LIMIT 50 NAMESPACE (\"_audit.org-a\", \"_audit.platform\")");
        assert!(vars.is_none());
        let q = Query { orgs: vec!["org-a".into()], like: Some("who deleted flows".into()), limit: 9999, ..Default::default() };
        let (aql, vars) = recall_statement(&q);
        assert_eq!(aql, "RECALL FROM EPISODIC LIKE $q LIMIT 500 NAMESPACE \"_audit.org-a\"");
        assert_eq!(vars.unwrap()["q"], "who deleted flows");
        let q = Query { agent: Some("agt-1".into()), since_ms: Some(5), ..Default::default() };
        assert!(matches(&q, &json!({ "agent": "agt-1", "ts_ms": 5 })));
        assert!(!matches(&q, &json!({ "agent": "agt-2", "ts_ms": 9 })));
        assert!(!matches(&q, &json!({ "agent": "agt-1", "ts_ms": 4 })));
    }

    #[test]
    fn a_row_reads_back_as_the_decision() {
        let row = json!({ "key": "pod-1", "data": { "key": "pod-1", "ts_ms": 5, "decision": "allow", "roles": "A,B", "agent": "agt-1" } });
        let v = from_row(&row);
        assert_eq!(v["id"], "pod-1");
        assert_eq!(v["ts_ms"], 5);
        assert_eq!(v["roles"], json!(["A", "B"]));
        assert_eq!(v["agent"], "agt-1");
    }

    /// AUD-4: a record hashed before it is written hashes the same when read back,
    /// and any edit changes the hash.
    #[test]
    fn a_chained_record_hashes_the_same_read_back() {
        let mut r = rec();
        r.org = Some("org-acme".into());
        let mut chains = HashMap::new();
        link(&mut chains, "pod@1", &mut r);
        let mut r2 = rec();
        r2.id = "pod-2".into();
        r2.org = Some("org-acme".into());
        link(&mut chains, "pod@1", &mut r2);
        assert_eq!((r.seq, r2.seq), (Some(1), Some(2)));
        assert_eq!(r2.prev_hash, r.hash);
        // As ADB holds it: the STORE fields, read back through from_row.
        let stored = |r: &AuditRecord| {
            let mut d = serde_json::Map::new();
            for (k, v) in [("key", json!(r.id)), ("ts_ms", json!(r.ts_ms)), ("pod", json!(r.pod)), ("principal_kind", json!(r.principal_kind)),
                ("principal_id", json!(r.principal_id)), ("on_behalf_of", json!(r.on_behalf_of)), ("action", json!(r.action)),
                ("resource_type", json!(r.resource_type)), ("resource_id", json!(r.resource_id)), ("decision", json!(r.decision)),
                ("reason", json!(r.reason)), ("project", json!(r.project)), ("org", json!(r.org)), ("policy_ids", json!(r.policy_ids.join(","))),
                ("roles", json!(r.roles.join(","))), ("agent", json!(r.agent)), ("policy_version", json!(r.policy_version)),
                ("chain", json!(r.chain)), ("seq", json!(r.seq)), ("prev_hash", json!(r.prev_hash)), ("hash", json!(r.hash))] {
                if !v.is_null() && v != json!("") { d.insert(k.into(), v); }
            }
            from_row(&json!({ "data": d }))
        };
        let back = stored(&r2);
        assert_eq!(back["hash"].as_str(), Some(hash_of(&back).as_str()), "read back, it checks");
        let mut edited = back.clone();
        edited["decision"] = json!("allow");
        assert_ne!(edited["hash"].as_str(), Some(hash_of(&edited).as_str()), "an edit shows");
    }
}
