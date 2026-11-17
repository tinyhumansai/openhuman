use super::*;
use crate::core::events::DomainEvent;
use tinybus::EventHandler;

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
