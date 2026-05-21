//! Broadcast bus for companion state change events.
//!
//! Follows the same pattern as `overlay::bus`: a process-global
//! `tokio::sync::broadcast` channel so any module can subscribe.
//! The Socket.IO bridge (PR 2) will forward these to the overlay
//! as `companion:state_changed` events.

use once_cell::sync::Lazy;
use tokio::sync::broadcast;

use super::types::CompanionStateChangedEvent;

const LOG_PREFIX: &str = "[desktop_companion]";

static STATE_BUS: Lazy<broadcast::Sender<CompanionStateChangedEvent>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(64);
    tx
});

/// Subscribe to companion state change events.
pub fn subscribe_state_changed() -> broadcast::Receiver<CompanionStateChangedEvent> {
    STATE_BUS.subscribe()
}

/// Publish a state change event.
///
/// Fire-and-forget: if nobody is subscribed the event is dropped.
pub fn publish_state_changed(event: CompanionStateChangedEvent) -> usize {
    log::debug!(
        "{LOG_PREFIX} state_changed session={} {} -> {}",
        event.session_id,
        event.previous_state,
        event.state,
    );
    match STATE_BUS.send(event) {
        Ok(n) => n,
        Err(_) => {
            log::debug!("{LOG_PREFIX} no subscribers — state change dropped");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::desktop_companion::types::CompanionState;

    #[tokio::test]
    async fn publish_is_received_by_subscriber() {
        // STATE_BUS is process-global — other tests may publish events.
        // We filter by session_id to avoid flakiness.
        let mut rx = subscribe_state_changed();
        let delivered = publish_state_changed(CompanionStateChangedEvent {
            session_id: "bus-test-unique".into(),
            state: CompanionState::Listening,
            previous_state: CompanionState::Idle,
            message: None,
        });
        assert!(delivered >= 1);
        // Drain until we find our specific event (others may have been published concurrently).
        loop {
            let event = rx.recv().await.expect("event delivered");
            if event.session_id == "bus-test-unique" {
                assert_eq!(event.state, CompanionState::Listening);
                assert_eq!(event.previous_state, CompanionState::Idle);
                break;
            }
        }
    }

    #[test]
    fn publish_with_no_subscribers_is_safe() {
        let _ = publish_state_changed(CompanionStateChangedEvent {
            session_id: "test".into(),
            state: CompanionState::Idle,
            previous_state: CompanionState::Error,
            message: Some("recovered".into()),
        });
    }
}
