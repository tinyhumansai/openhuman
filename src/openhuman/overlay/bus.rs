//! Broadcast bus for overlay attention events.
//!
//! Mirrors the pattern used by `voice::dictation_listener`: a single
//! `tokio::sync::broadcast` channel wrapped in a `Lazy` static so any
//! module in the core can publish without threading a sender around.
//! The Socket.IO bridge in `core::socketio::spawn_web_channel_bridge`
//! subscribes here and forwards every event to the overlay window as
//! an `overlay:attention` Socket.IO message.

use once_cell::sync::Lazy;
use tokio::sync::broadcast;

use super::types::OverlayAttentionEvent;

const LOG_PREFIX: &str = "[overlay]";

static ATTENTION_BUS: Lazy<broadcast::Sender<OverlayAttentionEvent>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(64);
    tx
});

/// Subscribe to overlay attention events. Used by the Socket.IO bridge.
pub fn subscribe_attention_events() -> broadcast::Receiver<OverlayAttentionEvent> {
    ATTENTION_BUS.subscribe()
}

/// Publish an attention event toward the overlay window.
///
/// Fire-and-forget: if nobody is currently subscribed (e.g. the bridge
/// hasn't started yet, or the overlay socket is disconnected) the event
/// is dropped. Returns the number of active subscribers that received
/// the event for diagnostics.
pub fn publish_attention(event: OverlayAttentionEvent) -> usize {
    log::debug!(
        "{LOG_PREFIX} publish attention source={:?} tone={:?} message_bytes={} ttl_ms={:?}",
        event.source,
        event.tone,
        event.message.len(),
        event.ttl_ms
    );
    match ATTENTION_BUS.send(event) {
        Ok(n) => n,
        Err(_) => {
            log::debug!("{LOG_PREFIX} no overlay subscribers — attention event dropped");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::overlay::types::OverlayAttentionTone;

    #[tokio::test]
    async fn publish_is_received_by_subscriber() {
        let mut rx = subscribe_attention_events();
        let delivered = publish_attention(
            OverlayAttentionEvent::new("hello overlay")
                .with_tone(OverlayAttentionTone::Accent)
                .with_source("test"),
        );
        assert!(delivered >= 1);
        let event = rx.recv().await.expect("event delivered");
        assert_eq!(event.message, "hello overlay");
        assert_eq!(event.tone, OverlayAttentionTone::Accent);
        assert_eq!(event.source.as_deref(), Some("test"));
    }

    #[test]
    fn publish_with_no_subscribers_is_safe() {
        // Drop any existing subscribers by not holding one.
        let _ = publish_attention(OverlayAttentionEvent::new("dropped"));
    }
}
