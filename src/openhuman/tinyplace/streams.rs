//! Tinyplace WebSocket stream manager.
//!
//! Holds active WebSocket connections keyed by a stream ID string. Each
//! connection is driven by a spawned tokio task that loops `recv()` and
//! publishes events to the global event bus.
//!
//! ## Lifecycle
//!
//! 1. Caller invokes `start_stream(kind, target_id, client)`.
//! 2. The manager derives a stable `stream_id`, checks the cap, then spawns a
//!    `recv_loop` task that calls `ws_handle.connect().await` and loops.
//! 3. Each message is published as `DomainEvent::TinyPlaceStreamMessage`.
//! 4. Status transitions are published as `DomainEvent::TinyPlaceStreamStatusChanged`.
//! 5. `stop_stream` / `stop_all` abort the spawned task and clean up the registry.
//!
//! **No auto-reconnect in v1.** If the WS drops the stream enters `disconnected`
//! status and the frontend can call `start` again.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::core::event_bus::{publish_global, DomainEvent};

const LOG_PREFIX: &str = "[tinyplace::streams]";

/// Maximum number of concurrent tinyplace WebSocket streams.
pub(crate) const MAX_CONCURRENT_STREAMS: usize = 5;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Stream kinds supported in v1.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StreamKind {
    Inbox,
    Conversation,
}

/// Metadata for an active stream (returned to the renderer).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StreamEntry {
    pub stream_id: String,
    pub kind: StreamKind,
    pub status: StreamStatus,
}

/// Lifecycle status of a stream.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StreamStatus {
    Connecting,
    Connected,
    Disconnected,
    Failed,
}

impl StreamStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
            Self::Failed => "failed",
        }
    }
}

/// Internal handle for a running stream task.
pub(crate) struct ActiveStream {
    pub(crate) kind: StreamKind,
    pub(crate) task: JoinHandle<()>,
    /// The original target id (e.g. conversation_id). Empty for inbox.
    #[allow(dead_code)]
    pub(crate) target_id: String,
}

// ── Stream ID helpers ─────────────────────────────────────────────────────────

/// Compute the canonical stream_id for a given kind + target.
pub(crate) fn stream_id_for(kind: StreamKind, target_id: Option<&str>) -> String {
    match kind {
        StreamKind::Inbox => "inbox".to_string(),
        StreamKind::Conversation => format!("conversation:{}", target_id.unwrap_or("")),
    }
}

// ── StreamManager ─────────────────────────────────────────────────────────────

/// Process-global stream registry.
pub(crate) struct StreamManager {
    pub(crate) streams: Mutex<HashMap<String, ActiveStream>>,
}

impl StreamManager {
    /// Build a new, empty manager.
    pub(crate) fn new() -> Self {
        Self {
            streams: Mutex::new(HashMap::new()),
        }
    }

    /// Start a WebSocket stream. Returns the stable `stream_id`.
    ///
    /// Idempotent: if the stream_id already exists, returns it immediately.
    /// Returns `Err` if the registry is at `MAX_CONCURRENT_STREAMS`.
    pub(crate) async fn start_stream(
        &self,
        kind: StreamKind,
        target_id: Option<String>,
        client: &tinyplace::TinyPlaceClient,
    ) -> Result<String, String> {
        let stream_id = stream_id_for(kind.clone(), target_id.as_deref());

        {
            let map = self.streams.lock().await;
            // Idempotent: already running.
            if map.contains_key(&stream_id) {
                log::debug!(
                    "{LOG_PREFIX} start_stream stream_id={stream_id} already active — returning existing"
                );
                return Ok(stream_id);
            }
            // Cap check.
            if map.len() >= MAX_CONCURRENT_STREAMS {
                return Err(format!(
                    "tinyplace stream cap reached ({MAX_CONCURRENT_STREAMS} concurrent streams max)"
                ));
            }
        }

        // Build the WebSocket handle from the SDK.
        let ws_handle = match &kind {
            StreamKind::Inbox => client.inbox.stream(),
            StreamKind::Conversation => {
                let conv_id = target_id.as_deref().unwrap_or("");
                client.conversations.stream(conv_id, None, None)
            }
        };

        // Publish connecting status.
        let kind_str = kind_to_str(&kind);
        publish_status(&stream_id, "connecting");

        log::debug!(
            "{LOG_PREFIX} start_stream stream_id={stream_id} kind={kind_str} — spawning recv_loop"
        );

        // Spawn the recv loop.
        let task = {
            let stream_id_task = stream_id.clone();
            let kind_task = kind.clone();
            tokio::spawn(async move {
                recv_loop(ws_handle, stream_id_task, kind_task).await;
            })
        };

        // Register in the map.
        {
            let mut map = self.streams.lock().await;
            map.insert(
                stream_id.clone(),
                ActiveStream {
                    kind,
                    task,
                    target_id: target_id.unwrap_or_default(),
                },
            );
        }

        Ok(stream_id)
    }

    /// Stop a stream by id. Idempotent — if not found, returns `Ok(())`.
    pub(crate) async fn stop_stream(&self, stream_id: &str) -> Result<(), String> {
        let entry = {
            let mut map = self.streams.lock().await;
            map.remove(stream_id)
        };
        match entry {
            None => {
                log::debug!("{LOG_PREFIX} stop_stream stream_id={stream_id} not found — no-op");
                Ok(())
            }
            Some(active) => {
                active.task.abort();
                publish_status(stream_id, "disconnected");
                log::debug!("{LOG_PREFIX} stop_stream stream_id={stream_id} aborted");
                Ok(())
            }
        }
    }

    /// Stop all active streams. Called during app shutdown.
    pub(crate) async fn stop_all(&self) {
        let mut map = self.streams.lock().await;
        for (stream_id, active) in map.drain() {
            active.task.abort();
            publish_status(&stream_id, "disconnected");
            log::debug!("{LOG_PREFIX} stop_all aborted stream_id={stream_id}");
        }
    }

    /// List metadata for all active streams.
    pub(crate) async fn list_streams(&self) -> Vec<StreamEntry> {
        let map = self.streams.lock().await;
        map.iter()
            .map(|(stream_id, active)| {
                let status = if active.task.is_finished() {
                    StreamStatus::Disconnected
                } else {
                    StreamStatus::Connected
                };
                StreamEntry {
                    stream_id: stream_id.clone(),
                    kind: active.kind.clone(),
                    status,
                }
            })
            .collect()
    }
}

// ── Global accessor ───────────────────────────────────────────────────────────

static GLOBAL_STREAM_MANAGER: std::sync::OnceLock<StreamManager> = std::sync::OnceLock::new();

/// Return the process-global [`StreamManager`], lazy-initialised on first call.
pub(crate) fn global_stream_manager() -> &'static StreamManager {
    GLOBAL_STREAM_MANAGER.get_or_init(StreamManager::new)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn kind_to_str(kind: &StreamKind) -> &'static str {
    match kind {
        StreamKind::Inbox => "inbox",
        StreamKind::Conversation => "conversation",
    }
}

fn publish_status(stream_id: &str, status: &'static str) {
    publish_global(DomainEvent::TinyPlaceStreamStatusChanged {
        stream_id: stream_id.to_string(),
        status: status.to_string(),
    });
}

/// The per-stream async receive loop. Connects then loops over `recv()`.
/// Cleans itself up from the registry on exit (best-effort; `stop_stream`
/// already handles the authoritative remove via `abort()`).
async fn recv_loop(
    ws_handle: tinyplace::websocket::TinyPlaceWebSocket,
    stream_id: String,
    kind: StreamKind,
) {
    let kind_str = kind_to_str(&kind).to_string();

    // Connect.
    log::debug!("{LOG_PREFIX} recv_loop stream_id={stream_id} connecting...");
    let mut connection = match ws_handle.connect().await {
        Ok(conn) => conn,
        Err(e) => {
            log::warn!("{LOG_PREFIX} recv_loop stream_id={stream_id} connect failed: {e}");
            publish_status(&stream_id, StreamStatus::Failed.as_str());
            // Self-remove from the registry so a new start is possible.
            if let Some(mgr) = GLOBAL_STREAM_MANAGER.get() {
                mgr.streams.lock().await.remove(&stream_id);
            }
            return;
        }
    };

    publish_status(&stream_id, StreamStatus::Connected.as_str());
    log::debug!("{LOG_PREFIX} recv_loop stream_id={stream_id} connected");

    // Message loop.
    loop {
        match connection.recv().await {
            Some(Ok(value)) => {
                log::trace!("{LOG_PREFIX} recv_loop stream_id={stream_id} message received");
                publish_global(DomainEvent::TinyPlaceStreamMessage {
                    stream_id: stream_id.clone(),
                    kind: kind_str.clone(),
                    message: value,
                });
            }
            Some(Err(e)) => {
                log::warn!("{LOG_PREFIX} recv_loop stream_id={stream_id} error: {e}");
                publish_status(&stream_id, StreamStatus::Failed.as_str());
                break;
            }
            None => {
                log::debug!(
                    "{LOG_PREFIX} recv_loop stream_id={stream_id} server closed connection"
                );
                publish_status(&stream_id, StreamStatus::Disconnected.as_str());
                break;
            }
        }
    }

    // Self-remove from the registry (best-effort; abort already removed it if
    // stop_stream was called first).
    if let Some(mgr) = GLOBAL_STREAM_MANAGER.get() {
        mgr.streams.lock().await.remove(&stream_id);
    }

    log::debug!("{LOG_PREFIX} recv_loop stream_id={stream_id} exited");
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stream_id_formatting() {
        assert_eq!(stream_id_for(StreamKind::Inbox, None), "inbox");
        assert_eq!(
            stream_id_for(StreamKind::Conversation, Some("abc123")),
            "conversation:abc123"
        );
    }

    #[tokio::test]
    async fn list_streams_empty_on_init() {
        let mgr = StreamManager::new();
        let entries = mgr.list_streams().await;
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn stop_nonexistent_stream_is_ok() {
        let mgr = StreamManager::new();
        let result = mgr.stop_stream("nonexistent").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn max_concurrent_streams_enforced() {
        let mgr = StreamManager::new();
        // Simulate filling the map with dummy entries.
        {
            let mut map = mgr.streams.lock().await;
            for i in 0..MAX_CONCURRENT_STREAMS {
                map.insert(
                    format!("dummy-{i}"),
                    ActiveStream {
                        kind: StreamKind::Inbox,
                        task: tokio::spawn(async {}),
                        target_id: String::new(),
                    },
                );
            }
        }
        // The map is now at capacity.
        let map = mgr.streams.lock().await;
        assert_eq!(map.len(), MAX_CONCURRENT_STREAMS);
    }

    #[tokio::test]
    async fn stop_all_clears_registry() {
        let mgr = StreamManager::new();
        {
            let mut map = mgr.streams.lock().await;
            map.insert(
                "test-1".to_string(),
                ActiveStream {
                    kind: StreamKind::Inbox,
                    task: tokio::spawn(async {}),
                    target_id: String::new(),
                },
            );
        }
        mgr.stop_all().await;
        let entries = mgr.list_streams().await;
        assert!(entries.is_empty());
    }
}
