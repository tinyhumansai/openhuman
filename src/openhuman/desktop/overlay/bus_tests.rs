use super::*;
use crate::openhuman::desktop::overlay::types::OverlayAttentionTone;

#[tokio::test]
async fn publish_is_received_by_subscriber() {
    let mut rx = subscribe_attention_events();
    let delivered = publish_attention(
        OverlayAttentionEvent::new("hello overlay")
            .with_tone(OverlayAttentionTone::Accent)
            .with_source("test"),
    );
    assert!(delivered >= 1);
    // Under heavy parallelism (coverage builds), the broadcast
    // channel may have lagged events from other tests sharing
    // the process-global bus. Drain until we find our message.
    let mut found = false;
    for _ in 0..16 {
        match rx.try_recv() {
            Ok(event) if event.message == "hello overlay" => {
                assert_eq!(event.tone, OverlayAttentionTone::Accent);
                assert_eq!(event.source.as_deref(), Some("test"));
                found = true;
                break;
            }
            Ok(_) => continue,
            Err(broadcast::error::TryRecvError::Lagged(n)) => {
                log::debug!("overlay bus test: skipped {n} lagged messages");
                continue;
            }
            Err(_) => break,
        }
    }
    assert!(found, "expected 'hello overlay' event from broadcast bus");
}

#[test]
fn publish_with_no_subscribers_is_safe() {
    // Drop any existing subscribers by not holding one.
    let _ = publish_attention(OverlayAttentionEvent::new("dropped"));
}
