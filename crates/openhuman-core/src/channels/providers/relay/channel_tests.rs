use super::*;
use std::sync::Arc;

fn sink() -> RelayChannel {
    RelayChannel::new(
        "telegram",
        "gw-1",
        "channel:telegram/42/-100",
        "req-1",
        "m-1",
    )
}

#[test]
fn the_outbound_event_names_the_chat_and_carries_the_text() {
    let event = sink().outbound_event(&SendMessage::new("hi there", "-100"));
    assert_eq!(event.event_name(), CHANNEL_OUTBOUND_EVENT);
    assert_eq!(event.context().client_id, "gw-1");
    assert_eq!(event.context().thread_id, "channel:telegram/42/-100");
    assert_eq!(event.context().request_id, "req-1");
    let (full_response, structured) = match event {
        crate::web_chat::WebChannelEvent::ChannelOutbound {
            full_response,
            structured,
            ..
        } => (full_response, structured),
        other => panic!("expected channel_outbound event, got {other:?}"),
    };
    assert_eq!(full_response.as_deref(), Some("hi there"));
    let structured = structured.expect("structured");
    assert_eq!(structured["kind"], CHANNEL_OUTBOUND_EVENT);
    assert_eq!(structured["channel"], "telegram");
    assert_eq!(structured["chat_id"], "-100");
    assert_eq!(structured["reply_to_message_id"], "m-1");

    let wire = serde_json::to_value(sink().outbound_event(&SendMessage::new("x", "-100")))
        .expect("serialize");
    assert!(
        wire.get("agent").is_none(),
        "the routing stamp is not on the wire"
    );
}

#[tokio::test]
async fn send_publishes_under_the_callers_stamp_and_records_the_text() {
    let mut events = crate::web_chat::subscribe_web_channel_events();
    let context =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None)
            .derive_with(
                crate::core::runtime::ContextOverlay::new(
                    crate::config::Config::default(),
                    crate::core::runtime::DomainSet::full(),
                    Default::default(),
                )
                .session_agent("u-relay-test")
                .profile("p-relay-test"),
            );
    let channel = Arc::new(RelayChannel::new(
        "telegram",
        "gw-stamp",
        "channel:telegram/1/2",
        "req-stamp",
        "m-9",
    ));
    let sender = Arc::clone(&channel);
    crate::core::runtime::CoreContext::scope(context, async move {
        sender
            .send(&SendMessage::new("reply one", "2"))
            .await
            .expect("send");
    })
    .await;

    let event = loop {
        let event = events.recv().await.expect("event");
        if event.context().request_id == "req-stamp" {
            break event;
        }
    };
    assert_eq!(event.event_name(), CHANNEL_OUTBOUND_EVENT);
    assert!(event.belongs_to("u-relay-test"));
    assert!(!event.belongs_to("u-someone-else"));
    assert!(event.belongs_to_profile("p-relay-test"));
    assert!(!event.belongs_to_profile("p-someone-else"));
    assert_eq!(channel.sent(), vec!["reply one".to_string()]);
}

#[test]
fn a_relay_does_not_support_draft_updates() {
    assert_eq!(sink().name(), "telegram");
}
