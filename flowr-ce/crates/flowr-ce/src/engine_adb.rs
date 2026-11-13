//! FlowR's engine store is ADB (ADB `.ai/H_ENGINE_STORE.md`, stage H):
//!
//! - **Embedded** (CE, and any process without an ADB server): ADB's durable
//!   engine runs in this process, its data in `<data dir>/adb`. One engine per
//!   directory per process — opening the same directory again shares it.
//! - **Remote** (EE): an ADB server over Arrow Flight, `FLOWR_ENGINE_ADB_URL`
//!   (its Flight endpoint) with the engine token `FLOWR_ENGINE_ADB_TOKEN`.
//!
//! Either way the store is engine space `FLOWR_ENGINE_SPACE` (default
//! `default`; `auto` derives one from the data directory, for tests sharing one
//! ADB), spoken with `flowr_storage::engine_wire` — the same wire as FlowR's
//! own gateway.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde_json::{json, Value};
use tracing::warn;

use flowr_storage::engine::{EngineStore, StoreError, StoreResult};
use flowr_storage::engine_wire::{unwrap, Client, Transport};

/// The engine space this process uses.
pub fn space(data_dir: &str) -> String {
    match std::env::var("FLOWR_ENGINE_SPACE").ok().filter(|s| !s.is_empty()).as_deref() {
        Some("auto") => {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            data_dir.hash(&mut h);
            format!("t{:016x}", h.finish())
        }
        Some(s) => s.to_string(),
        None => "default".to_string(),
    }
}

/// ADB's engine in this process, for `data_dir` (shared by every storage the
/// process opens on that directory).
pub async fn embedded(data_dir: &str) -> anyhow::Result<Arc<dyn EngineStore>> {
    let (engine, key) = local(data_dir).await?;
    let store = engine.engine_store(&space(data_dir)).ok_or_else(|| anyhow::anyhow!("ADB engine at {key} has no engine store"))?;
    Ok(Arc::new(store))
}

/// The ADB engine in this process for `data_dir` (`<data_dir>/adb`), opened
/// once: runs, and in CE the registry and state (platform tables), share it.
pub async fn local(data_dir: &str) -> anyhow::Result<(Arc<adb_executor::LocalEngine>, String)> {
    static ENGINES: OnceLock<tokio::sync::Mutex<HashMap<String, Arc<adb_executor::LocalEngine>>>> = OnceLock::new();
    let dir = std::path::Path::new(data_dir).join("adb");
    let key = dir.to_string_lossy().to_string();
    let mut engines = ENGINES.get_or_init(Default::default).lock().await;
    let engine = match engines.get(&key) {
        Some(e) => Arc::clone(e),
        None => {
            let (e, stats) = adb_executor::LocalEngine::durable(&dir).await.map_err(|e| anyhow::anyhow!("ADB engine at {key}: {e}"))?;
            tracing::info!(path = %key, records = stats.records, "engine store: ADB (embedded)");
            let e = Arc::new(e);
            engines.insert(key.clone(), Arc::clone(&e));
            e
        }
    };
    Ok((engine, key))
}

/// The ADB server's engine store, if `FLOWR_ENGINE_ADB_URL` is set.
pub fn remote_from_env(data_dir: &str) -> Option<Client<FlightTransport>> {
    let url = std::env::var("FLOWR_ENGINE_ADB_URL").ok().filter(|u| !u.is_empty())?;
    let token = std::env::var("FLOWR_ENGINE_ADB_TOKEN").ok().filter(|t| !t.is_empty());
    match FlightTransport::new(&url, token, space(data_dir)) {
        Ok(t) => Some(Client::new(t)),
        Err(e) => {
            warn!(url, error = %e, "FLOWR_ENGINE_ADB_URL is not a usable Flight endpoint");
            None
        }
    }
}

/// A failure of the connection, not an answer from ADB. An HTTP/2 GOAWAY reaches the
/// caller as `Internal`, so a call must recognise it by its message.
fn retriable(e: &tonic::Status) -> bool {
    matches!(e.code(), tonic::Code::Unavailable | tonic::Code::Unknown | tonic::Code::Cancelled | tonic::Code::DeadlineExceeded) || protocol_error(e)
}

/// An HTTP/2 protocol error on the connection (PF-41): retried, and after
/// [`PROTOCOL_STRIKES`] in a row on one node its channel is replaced, since the old
/// connection can keep failing every call made on it.
fn protocol_error(e: &tonic::Status) -> bool {
    e.code() == tonic::Code::Internal && e.message().contains("h2 protocol error")
}

const PROTOCOL_STRIKES: u32 = 2;

/// A status with its source chain, for the log (the message alone said only
/// "h2 protocol error: http2 error").
fn status_detail(e: &tonic::Status) -> String {
    let mut out = format!("{:?}: {}", e.code(), e.message());
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        out.push_str(&format!(" <- {s}"));
        src = s.source();
    }
    out
}

fn connect(u: &str) -> Result<tonic::transport::Channel, String> {
    Ok(tonic::transport::Channel::from_shared(u.to_string()).map_err(|e| format!("bad ADB Flight URL {u}: {e}"))?.connect_timeout(Duration::from_secs(5)).connect_lazy())
}

/// The engine wire over ADB's Flight: `do_action("engine")` per op,
/// `do_action("engine_subscribe")` for notices.
pub struct FlightTransport {
    url: String,
    /// C-8: every ADB node's Flight address (`FLOWR_ENGINE_ADB_URL` may list them, comma
    /// separated): only the leader serves Flight, so a call that finds a node unavailable
    /// moves on to the next, and stays with the one that answers.
    channels: Arc<Vec<std::sync::RwLock<tonic::transport::Channel>>>,
    /// Each node's URL, to open a fresh channel (PF-41), and its protocol errors in a row.
    urls: Arc<Vec<String>>,
    strikes: Arc<Vec<std::sync::atomic::AtomicU32>>,
    /// Notice streams (`subscribe`) on connections of their own (PF-41): a run's stream
    /// is reset when the run ends, and once 1,024 resets pile up on one HTTP/2
    /// connection h2 fails it — with every engine call on it.
    sub_channels: Arc<Vec<tonic::transport::Channel>>,
    current: Arc<std::sync::atomic::AtomicUsize>,
    bearer: Option<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>,
    space: String,
}

impl FlightTransport {
    pub fn new(url: &str, token: Option<String>, space: String) -> Result<Self, String> {
        let urls: Vec<String> = url.split(',').map(str::trim).filter(|u| !u.is_empty()).map(String::from).collect();
        let mut channels = Vec::new();
        for u in &urls {
            channels.push(std::sync::RwLock::new(connect(u)?));
        }
        let strikes = urls.iter().map(|_| std::sync::atomic::AtomicU32::new(0)).collect();
        let sub_channels = urls.iter().map(|u| connect(u)).collect::<Result<Vec<_>, _>>()?;
        if channels.is_empty() {
            return Err("no ADB Flight URL".into());
        }
        let bearer = match token {
            Some(t) => Some(format!("Bearer {t}").parse().map_err(|_| "the engine token is not a valid header".to_string())?),
            None => None,
        };
        Ok(Self { url: url.to_string(), channels: Arc::new(channels), urls: Arc::new(urls), strikes: Arc::new(strikes), sub_channels: Arc::new(sub_channels), current: Default::default(), bearer, space })
    }

    fn request<T>(&self, body: T) -> tonic::Request<T> {
        let mut r = tonic::Request::new(body);
        if let Some(b) = &self.bearer {
            r.metadata_mut().insert("authorization", b.clone());
        }
        r
    }

    /// The node to call now.
    fn channel(&self) -> tonic::transport::Channel {
        let i = self.current.load(std::sync::atomic::Ordering::Relaxed) % self.channels.len();
        self.channels[i].read().unwrap().clone()
    }

    /// A call on node `i` answered (or failed otherwise): its protocol errors start over.
    fn healthy(&self, i: usize) {
        self.strikes[i].store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// A protocol error on node `i` (PF-41): logged with its cause; after
    /// [`PROTOCOL_STRIKES`] in a row the node gets a new channel (a new connection).
    fn protocol_strike(&self, i: usize, op: &str, e: &tonic::Status) {
        let n = self.strikes[i].fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        warn!(node = %self.urls[i], op, strikes = n, error = %status_detail(e), "HTTP/2 protocol error on the ADB engine connection");
        if n >= PROTOCOL_STRIKES {
            if let Ok(c) = connect(&self.urls[i]) {
                *self.channels[i].write().unwrap() = c;
                self.strikes[i].store(0, std::sync::atomic::Ordering::Relaxed);
                warn!(node = %self.urls[i], "ADB engine channel replaced after repeated HTTP/2 protocol errors");
            }
        }
    }

    /// The node `failed` did not answer: try the next (another caller may have moved on).
    fn next_node(&self, failed: usize) {
        let n = self.channels.len();
        if n > 1 {
            let _ = self.current.compare_exchange(failed, (failed + 1) % n, std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn client(&self) -> arrow_flight::flight_service_client::FlightServiceClient<tonic::transport::Channel> {
        arrow_flight::flight_service_client::FlightServiceClient::new(self.channel())
            .max_decoding_message_size(64 << 20)
            .max_encoding_message_size(64 << 20)
    }
}

impl FlightTransport {
    async fn call_raw(&self, op: &str, args: Value) -> StoreResult<Value> {
        let body = json!({ "space": self.space, "op": op, "args": args }).to_string();
        // Retried briefly, like the gateway client: ADB restarting must not fail a run's write.
        let mut wait = Duration::from_millis(200);
        let nodes = self.channels.len();
        // C-21: retried for as long as an election takes (bounded), not a fixed count.
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        for attempt in 0.. {
            let last = std::time::Instant::now() >= deadline;
            let on = self.current.load(std::sync::atomic::Ordering::Relaxed) % nodes;
            // C-8: the first pass over the other nodes goes without the backoff sleep.
            let hop = |this: &Self| { this.next_node(on); attempt + 1 < nodes };
            let action = arrow_flight::Action { r#type: "engine".into(), body: body.clone().into() };
            // A call on a connection that broke must fail and be retried, not hang.
            let mut req = self.request(action);
            req.set_timeout(Duration::from_secs(30));
            match self.client().do_action(req).await {
                Ok(r) => {
                    let mut s = r.into_inner();
                    let answer = match s.next().await {
                        Some(Ok(a)) => {
                            self.healthy(on);
                            a
                        }
                        // The connection can also break after the call was accepted.
                        Some(Err(e)) if retriable(&e) && !last => {
                            if protocol_error(&e) {
                                self.protocol_strike(on, op, &e);
                            }
                            if !hop(self) {
                                tokio::time::sleep(wait).await;
                                wait = (wait * 2).min(Duration::from_secs(2));
                            }
                            continue;
                        }
                        Some(Err(e)) => return Err(StoreError::Backend(format!("ADB {op}: {}", e.message()))),
                        None => return Err(StoreError::Backend(format!("ADB {op}: no answer"))),
                    };
                    // To the end: dropped here, the stream would be reset with ADB's
                    // trailers in flight (see `finish`).
                    crate::handlers::call_adb::finish(&mut s).await;
                    let v: Value = serde_json::from_slice(&answer.body).map_err(|e| StoreError::Backend(format!("ADB {op}: {e}")))?;
                    return unwrap(v);
                }
                Err(e) if matches!(e.code(), tonic::Code::Unauthenticated | tonic::Code::PermissionDenied) => {
                    return Err(StoreError::Backend(format!("ADB at {} refused the engine token: {}", self.url, e.message())));
                }
                // Cancelled: the HTTP/2 connection went away under the call (an ADB
                // restart or failover); the call did not reach ADB or is retried under
                // the same arguments, which the engine core treats idempotently.
                Err(e) if retriable(&e) && !last => {
                    if protocol_error(&e) {
                        self.protocol_strike(on, op, &e);
                    }
                    if !hop(self) {
                        tokio::time::sleep(wait).await;
                        wait = (wait * 2).min(Duration::from_secs(2));
                    }
                }
                Err(e) => return Err(StoreError::Backend(format!("ADB at {}: {}", self.url, status_detail(&e)))),
            }
        }
        unreachable!()
    }
}

#[async_trait]
impl Transport for FlightTransport {
    async fn call(&self, op: &str, args: Value) -> StoreResult<Value> {
        // AO-4: each engine store op, counted and timed (`flowr::engine` at debug).
        let t = flowr_metrics::Timer::start();
        let r = self.call_raw(op, args).await;
        flowr_metrics::observe(&flowr_metrics::get().engine_ops, &flowr_metrics::get().engine_seconds, &[op], r.is_ok(), t.took());
        if t.took() >= flowr_metrics::slow() {
            tracing::info!(target: "flowr::engine", op, ms = t.ms(), ok = r.is_ok(), "slow engine op");
        } else {
            tracing::debug!(target: "flowr::engine", op, ms = t.ms(), ok = r.is_ok(), "engine op");
        }
        r
    }



    /// Notices from ADB, reconnecting until the receiver is dropped (best effort).
    ///
    /// The stream to ADB is dropped as soon as the receiver is: a run's topic gets no
    /// more notices once it ends, and a stream kept open waiting for one leaked an
    /// HTTP/2 stream per run on the shared connection until new calls stalled.
    fn subscribe(&self, topic: &str) -> BoxStream<'static, Value> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
        let body = json!({ "space": self.space, "topic": topic }).to_string();
        let (channels, current, bearer) = (Arc::clone(&self.sub_channels), Arc::clone(&self.current), self.bearer.clone());
        tokio::spawn(async move {
            while !tx.is_closed() {
                // The node the calls use now (C-8): after a takeover, the new leader.
                let on = current.load(std::sync::atomic::Ordering::Relaxed) % channels.len();
                let mut client = arrow_flight::flight_service_client::FlightServiceClient::new(channels[on].clone());
                let mut req = tonic::Request::new(arrow_flight::Action { r#type: "engine_subscribe".into(), body: body.clone().into() });
                if let Some(b) = &bearer {
                    req.metadata_mut().insert("authorization", b.clone());
                }
                let opened = tokio::select! {
                    r = client.do_action(req) => r,
                    _ = tx.closed() => return,
                };
                if opened.is_err() && channels.len() > 1 {
                    let _ = current.compare_exchange(on, (on + 1) % channels.len(), std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed);
                }
                if let Ok(r) = opened {
                    let mut s = r.into_inner();
                    loop {
                        tokio::select! {
                            n = s.next() => match n {
                                Some(Ok(n)) => {
                                    if tx.send(serde_json::from_slice(&n.body).unwrap_or(Value::Null)).is_err() {
                                        return;
                                    }
                                }
                                _ => break,
                            },
                            // The receiver is gone: drop the stream (resets it on the connection).
                            _ = tx.closed() => return,
                        }
                    }
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    _ = tx.closed() => return,
                }
            }
        });
        futures::stream::poll_fn(move |cx| rx.poll_recv(cx)).boxed()
    }
}

/// The router sends runs only to pools with a live worker (stage 8). With an
/// ADB store, workers claim from ADB, not through this process: learn the live
/// pools from the store every few seconds.
pub(crate) fn spawn_live_pools(engine: Arc<dyn EngineStore>) {
    static STARTED: OnceLock<Mutex<bool>> = OnceLock::new();
    let mut started = STARTED.get_or_init(Default::default).lock().unwrap();
    if *started {
        return;
    }
    *started = true;
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            match engine.live_pools(Duration::from_secs(30)).await {
                Ok(pools) => pools.iter().for_each(|p| crate::engine_remote::mark_live(p)),
                Err(e) => tracing::debug!(error = %e, "live pools unavailable"),
            }
        }
    });
}
