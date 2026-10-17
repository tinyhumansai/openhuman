use super::*;
use crate::backend::BackendApiError;
use crate::core::events::DomainEvent;
use tinybus::EventHandler;

use crate::channels::bus::delivery::map_backend_error;
use tinychannels_bus::module_progressive::ProgressiveTransportResult;
#[test]
fn subscriber_metadata_is_stable() {
    let subscriber = ChannelInboundSubscriber::new();
    assert_eq!(subscriber.name(), "channel::inbound_handler");
    assert_eq!(subscriber.domains(), Some(&["channel"][..]));
}

#[tokio::test]
async fn unrelated_events_are_ignored() {
    ChannelInboundSubscriber
        .handle(&DomainEvent::SystemStartup {
            component: "test".into(),
        })
        .await;
}

// ── Backend error mapping (#5230) ───────────────────────────────────────────
//
// Route absence and message absence need different recoveries in the
// progressive driver; conflating them once left a permanent "💭 Thinking:"
// bubble in the chat. The mapping is by type, never by message text.

#[test]
fn backend_errors_map_to_distinct_progressive_recoveries() {
    let route_absent = anyhow::Error::new(BackendApiError::ChannelEditUnsupported {
        provider: "telegram".into(),
        message_id: "1103".into(),
    });
    assert!(matches!(
        map_backend_error(route_absent),
        ProgressiveTransportResult::EditUnsupported
    ));

    let message_absent = anyhow::Error::new(BackendApiError::MessageNotFound {
        provider: "telegram".into(),
        message_id: "1103".into(),
    });
    assert!(matches!(
        map_backend_error(message_absent),
        ProgressiveTransportResult::MessageGone
    ));

    // Anything untyped (5xx, transport, rate limit) keeps the retry budget.
    assert!(matches!(
        map_backend_error(anyhow::anyhow!("502 Bad Gateway")),
        ProgressiveTransportResult::Failed
    ));

    // A different typed variant must not be mistaken for either edit case.
    let unauthorized = anyhow::Error::new(BackendApiError::Unauthorized {
        method: "PATCH".into(),
        path: "/channels/telegram/messages/1103".into(),
    });
    assert!(matches!(
        map_backend_error(unauthorized),
        ProgressiveTransportResult::Failed
    ));
}
