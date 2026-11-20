//! MCP-over-SSE connections: one private channel per `GET /sse` stream.
//!
//! Each connection is registered under a random id together with the caller
//! who opened it. A reply posted for that id (`POST /sse?session_id=`) reaches
//! that connection only, and only when the poster is the same caller — no
//! stream ever sees another client's traffic, and a slow client can only fall
//! behind on its own replies.
//!
//! Cluster mode: the `POST` may land on a different FlowR than the `GET`. A
//! reply for a connection this process does not hold goes through the engine
//! store (ADB) on topic `mcp:<id>`, which only the process holding that
//! connection follows; that process checks the owner again before delivering.
//!
//! The id names its owner: `<uuid>.<tag>`, the tag a hash of the uuid and the
//! owner. Any FlowR can so tell, before running the request, that a poster is
//! not the owner (the poster is the authenticated caller, so the tag cannot be
//! made to match someone else's connection) and answer 404 as the holder would.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::json;
use tokio::sync::mpsc;

/// Replies buffered per connection before a poster waits for the client.
const CHANNEL_CAP: usize = 256;

struct Conn {
    owner: String,
    tx: mpsc::Sender<String>,
    follow: Option<tokio::task::AbortHandle>,
}

static CONNS: OnceLock<Mutex<HashMap<String, Conn>>> = OnceLock::new();
static CLUSTER: OnceLock<Arc<dyn flowr_storage::engine::EngineStore>> = OnceLock::new();

fn conns() -> &'static Mutex<HashMap<String, Conn>> {
    CONNS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The owner tag of connection `uuid`: binds the id to who opened it.
fn tag(uuid: &str, owner: &str) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(format!("mcp-sse|{uuid}|{owner}").as_bytes());
    h[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// Was connection `id` opened by `owner`? (Its tag says so.)
fn opened_by(id: &str, owner: &str) -> bool {
    id.split_once('.').is_some_and(|(uuid, t)| t == tag(uuid, owner))
}

/// The engine topic carrying replies for connection `id`.
pub fn topic(id: &str) -> String {
    format!("mcp:{id}")
}

/// Turn on cluster mode: replies for connections held elsewhere go through `engine`.
pub fn cluster(engine: Arc<dyn flowr_storage::engine::EngineStore>) {
    let _ = CLUSTER.set(engine);
}

/// An open connection. Dropping it (the client went away) unregisters it.
pub struct Connection {
    pub id: String,
    pub rx: mpsc::Receiver<String>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        let gone = conns().lock().ok().and_then(|mut m| m.remove(&self.id));
        if let Some(h) = gone.and_then(|c| c.follow) {
            h.abort();
        }
    }
}

/// Register a new connection for `owner` (the caller's principal id).
pub fn open(owner: &str) -> Connection {
    let uuid = uuid::Uuid::new_v4().to_string();
    let id = format!("{uuid}.{}", tag(&uuid, owner));
    let (tx, rx) = mpsc::channel(CHANNEL_CAP);
    let follow = CLUSTER.get().map(|engine| {
        use futures::StreamExt;
        let mut notices = engine.subscribe(&topic(&id));
        let (id, owner, tx) = (id.clone(), owner.to_string(), tx.clone());
        tokio::spawn(async move {
            while let Some(n) = notices.next().await {
                let from = n.get("owner").and_then(|o| o.as_str()).unwrap_or_default();
                let Some(msg) = n.get("msg").and_then(|m| m.as_str()) else { continue };
                if from != owner {
                    tracing::warn!(connection = %id, "MCP reply from another caller dropped");
                    continue;
                }
                if tx.send(msg.to_string()).await.is_err() {
                    break;
                }
            }
        })
        .abort_handle()
    });
    if let Ok(mut m) = conns().lock() {
        m.insert(id.clone(), Conn { owner: owner.to_string(), tx, follow });
    }
    Connection { id, rx }
}

/// Why a reply could not be routed.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// No such connection here (and no cluster to look further), or it is
    /// somebody else's — the two are indistinguishable to the caller.
    NotFound,
}

/// Is `id` a connection `owner` may post to? Checked before the request runs.
/// One held by another process (cluster mode) is checked by its tag here, and
/// by its owner again where it is held.
pub fn may_post(id: &str, owner: &str) -> Result<(), Refused> {
    if !opened_by(id, owner) {
        return Err(Refused::NotFound);
    }
    let m = conns().lock().map_err(|_| Refused::NotFound)?;
    match m.get(id) {
        Some(c) if c.owner == owner => Ok(()),
        Some(_) => Err(Refused::NotFound),
        None if CLUSTER.get().is_some() => Ok(()),
        None => Err(Refused::NotFound),
    }
}

/// Deliver `msg` to connection `id` on behalf of `owner`.
pub async fn deliver(id: &str, owner: &str, msg: String) -> Result<(), Refused> {
    if !opened_by(id, owner) {
        return Err(Refused::NotFound);
    }
    let local = {
        let m = conns().lock().map_err(|_| Refused::NotFound)?;
        match m.get(id) {
            Some(c) if c.owner == owner => Some(c.tx.clone()),
            Some(_) => return Err(Refused::NotFound),
            None => None,
        }
    };
    if let Some(tx) = local {
        return tx.send(msg).await.map_err(|_| Refused::NotFound);
    }
    let Some(engine) = CLUSTER.get() else { return Err(Refused::NotFound) };
    engine
        .publish(&topic(id), json!({ "owner": owner, "msg": msg }))
        .await
        .map_err(|e| {
            tracing::warn!(connection = %id, error = %e, "MCP reply not published to the cluster");
            Refused::NotFound
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_reply_reaches_only_its_own_connection() {
        let mut a = open("alice");
        let mut b = open("bob");
        deliver(&a.id, "alice", "for-a".into()).await.unwrap();
        assert_eq!(a.rx.recv().await.as_deref(), Some("for-a"));
        assert!(b.rx.try_recv().is_err(), "another connection must not see it");
    }

    #[tokio::test]
    async fn nobody_posts_to_someone_elses_connection() {
        let mut a = open("alice");
        assert_eq!(may_post(&a.id, "mallory"), Err(Refused::NotFound));
        assert_eq!(deliver(&a.id, "mallory", "x".into()).await, Err(Refused::NotFound));
        assert!(a.rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_closed_connection_is_forgotten() {
        let a = open("alice");
        let id = a.id.clone();
        drop(a);
        assert_eq!(may_post(&id, "alice"), Err(Refused::NotFound));
    }

    #[test]
    fn a_connection_id_names_its_owner() {
        let a = open("alice");
        assert!(opened_by(&a.id, "alice"));
        assert!(!opened_by(&a.id, "mallory"), "another caller");
        assert!(!opened_by("00000000-0000-4000-8000-000000000000", "alice"), "no tag");
        let (uuid, _) = a.id.split_once('.').unwrap();
        assert!(!opened_by(&format!("{uuid}.{}", "0".repeat(32)), "alice"), "a made-up tag");
    }
}
