use super::*;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use async_trait::async_trait;
use std::sync::Arc;
use tinybus::EventHandler;
use tokio::sync::Mutex as AsyncMutex;

#[derive(Default)]
struct Capture {
    events: Arc<AsyncMutex<Vec<VoiceEvent>>>,
}

#[async_trait]
impl EventHandler<DomainEvent> for Capture {
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
    crate::core::bus::init().await.expect("bus init");
    let capture = Capture::default();
    let events = capture.events.clone();
    let _sub = BUS.subscribe(Arc::new(capture));

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
