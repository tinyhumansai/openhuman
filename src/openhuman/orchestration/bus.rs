//! Event-bus subscriber that drives orchestration ingest off inbound tiny.place
//! DM stream messages.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use once_cell::sync::Lazy;
use tokio::sync::broadcast;

use crate::core::event_bus::{subscribe_global, DomainEvent, EventHandler, SubscriptionHandle};

static INGEST_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();
static WAKE_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Broadcast bus of orchestration chat activity for the renderer socket bridge
/// (stage 7). Each message is a `{ agentId, sessionId, chatKind }` payload the
/// `core/socketio.rs` bridge re-emits as `orchestration:message` so the
/// `TinyPlaceOrchestrationTab` can targeted-refetch the affected chat live.
static ORCH_SOCKET_BUS: Lazy<broadcast::Sender<serde_json::Value>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(128);
    tx
});

/// Subscribe to orchestration socket activity. Used by the Socket.IO bridge.
pub fn subscribe_orchestration_socket() -> broadcast::Receiver<serde_json::Value> {
    ORCH_SOCKET_BUS.subscribe()
}

/// Fan an orchestration chat activity event out to the renderer socket bridge.
pub fn notify_orchestration_message(agent_id: &str, session_id: &str, chat_kind: &str) {
    let payload = serde_json::json!({
        "agentId": agent_id,
        "sessionId": session_id,
        "chatKind": chat_kind,
    });
    // No subscribers (headless / cron) is fine — drop silently.
    let _ = ORCH_SOCKET_BUS.send(payload);
}

/// Register the orchestration ingest subscriber on the global event bus.
pub fn register_orchestration_ingest_subscriber() {
    if INGEST_HANDLE.get().is_some() {
        return;
    }
    match subscribe_global(Arc::new(OrchestrationIngestSubscriber)) {
        Some(handle) => {
            let _ = INGEST_HANDLE.set(handle);
        }
        None => {
            log::warn!(
                "[orchestration] failed to register ingest subscriber — bus not initialized"
            );
        }
    }
}

pub struct OrchestrationIngestSubscriber;

#[async_trait]
impl EventHandler for OrchestrationIngestSubscriber {
    fn name(&self) -> &str {
        "orchestration::ingest"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["tinyplace"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::TinyPlaceStreamMessage {
            stream_id,
            kind,
            message,
        } = event
        else {
            return;
        };
        let config = match crate::openhuman::config::Config::load_or_init().await {
            Ok(config) => config,
            Err(e) => {
                log::warn!("[orchestration] ingest.config_load_failed: {e}");
                return;
            }
        };
        super::ingest::ingest_stream_message(&config, kind, stream_id, message).await;
    }
}

/// Register the orchestration **wake** subscriber: on each persisted session DM
/// (`OrchestrationSessionMessage`, published by ingest), schedule a debounced
/// wake-graph run for that session (stage 4). Kept separate from the ingest
/// subscriber so the transport path stays independent of the graph path.
pub fn register_orchestration_wake_subscriber() {
    if WAKE_HANDLE.get().is_some() {
        return;
    }
    match subscribe_global(Arc::new(OrchestrationWakeSubscriber)) {
        Some(handle) => {
            let _ = WAKE_HANDLE.set(handle);
        }
        None => {
            log::warn!("[orchestration] failed to register wake subscriber — bus not initialized");
        }
    }
}

pub struct OrchestrationWakeSubscriber;

#[async_trait]
impl EventHandler for OrchestrationWakeSubscriber {
    fn name(&self) -> &str {
        "orchestration::wake"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["agent"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::OrchestrationSessionMessage {
            agent_id,
            session_id,
            chat_kind,
        } = event
        else {
            return;
        };
        // Live UI: fan every persisted chat message out to the renderer socket
        // (all kinds — session, master, subconscious) before the wake gating.
        notify_orchestration_message(agent_id, session_id, chat_kind);
        super::ops::schedule_wake(agent_id.clone(), session_id.clone(), chat_kind.clone()).await;
    }
}
