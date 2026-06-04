//! Voice domain event publishers. The PTT transcript-committed event is
//! published here so the future screen-intelligence follow-up can subscribe
//! and grab a frame on commit without coupling to the channel-web flow.

use crate::core::event_bus::{publish_global, DomainEvent, VoiceEvent};

/// Publish a [`VoiceEvent::PttTranscriptCommitted`] event.
pub fn publish_ptt_transcript_committed(
    thread_id: String,
    session_id: u64,
    text_len: usize,
    held_ms: u64,
    finalized_by_watchdog: bool,
) {
    publish_global(DomainEvent::Voice(VoiceEvent::PttTranscriptCommitted {
        thread_id,
        session_id,
        text_len,
        held_ms,
        finalized_by_watchdog,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::event_bus::{init_global, subscribe_global, DomainEvent, EventHandler};
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::sync::Mutex as AsyncMutex;

    #[derive(Default)]
    struct Capture {
        events: Arc<AsyncMutex<Vec<VoiceEvent>>>,
    }

    #[async_trait]
    impl EventHandler for Capture {
        fn name(&self) -> &str {
            "voice::ptt_test_capture"
        }

        fn domains(&self) -> Option<&[&str]> {
            Some(&["voice"])
        }

        async fn handle(&self, event: &DomainEvent) {
            if let DomainEvent::Voice(v) = event {
                self.events.lock().await.push(v.clone());
            }
        }
    }

    #[tokio::test]
    async fn publishing_a_ptt_commit_reaches_a_subscriber() {
        // Use the singleton (init is idempotent).
        let _ = init_global(64);
        let capture = Capture::default();
        let events = capture.events.clone();
        let _sub = subscribe_global(Arc::new(capture));

        publish_ptt_transcript_committed("thread-1".to_string(), 42, 17, 850, false);

        // Give the broadcaster a tick to deliver.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let got = events.lock().await;
        let found = got.iter().find_map(|e| match e {
            VoiceEvent::PttTranscriptCommitted {
                thread_id,
                session_id,
                text_len,
                held_ms,
                finalized_by_watchdog,
            } => Some((
                thread_id.clone(),
                *session_id,
                *text_len,
                *held_ms,
                *finalized_by_watchdog,
            )),
        });
        assert_eq!(
            found,
            Some(("thread-1".to_string(), 42, 17, 850, false)),
            "expected the published event to round-trip with all five fields; got events: {got:?}",
        );
    }
}
