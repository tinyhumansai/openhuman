use super::*;
use crate::channels::runtime::{ChannelHandle, TestChannel};
use std::sync::atomic::Ordering;

fn channel(name: &str, target: Option<&str>) -> (Arc<ChannelHandle>, Arc<TestChannel>) {
    let mut channel = TestChannel::new(name);
    channel.proactive_target = target.map(str::to_owned);
    let channel = Arc::new(channel);
    (Arc::new(ChannelHandle::Test(Arc::clone(&channel))), channel)
}

fn proactive_event() -> DomainEvent {
    DomainEvent::ProactiveMessageRequested {
        source: "cron:test".into(),
        message: "Hello!".into(),
        job_name: Some("test".into()),
        tenant: None,
    }
}

#[tokio::test]
async fn routes_to_active_external_channel() {
    let (ch, state) = channel("telegram", Some("telegram"));
    let map: HashMap<String, Arc<ChannelHandle>> = [("telegram".into(), ch)].into();
    let sub = ProactiveMessageSubscriber::new(Arc::new(map), Some("telegram".into()));

    sub.handle(&proactive_event()).await;

    assert_eq!(state.send_count.load(Ordering::SeqCst), 1);
    assert_eq!(state.outbound_intent_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn web_and_external_proactive_subscribers_deliver_each_destination_once() {
    let (ch, state) = channel("telegram", Some("telegram"));
    let map: HashMap<String, Arc<ChannelHandle>> = [("telegram".into(), ch)].into();
    let external = ProactiveMessageSubscriber::new(Arc::new(map), Some("telegram".into()));
    let web = crate::web_chat::proactive::WebProactiveSubscriber;
    let mut web_events = crate::web_chat::subscribe_web_channel_events();
    let event = proactive_event();

    web.handle(&event).await;
    external.handle(&event).await;

    let delivered = web_events.recv().await.expect("web event");
    assert_eq!(delivered.event_name(), "proactive_message");
    assert!(matches!(
        delivered.wire_event(),
        crate::web_chat::WebChannelEvent::ProactiveMessage {
            full_response: Some(response), ..
        } if response == "Hello!"
    ));
    assert!(
        web_events.try_recv().is_err(),
        "web event must not be duplicated"
    );
    assert_eq!(state.send_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn skips_external_when_channel_has_no_proactive_target() {
    // The active channel is the configured default, but it has no resolvable
    // delivery target (e.g. Telegram with no stored chat). Proactive routing
    // must skip it rather than calling `send` with an empty recipient
    // (#3794 review — Codex P2).
    let (ch, state) = channel("telegram", None);
    let map: HashMap<String, Arc<ChannelHandle>> = [("telegram".into(), ch)].into();
    let sub = ProactiveMessageSubscriber::new(Arc::new(map), Some("telegram".into()));

    sub.handle(&proactive_event()).await;

    assert_eq!(state.send_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn skips_external_when_active_is_web() {
    let (ch, state) = channel("telegram", Some("telegram"));
    let map: HashMap<String, Arc<ChannelHandle>> = [("telegram".into(), ch)].into();
    let sub = ProactiveMessageSubscriber::new(Arc::new(map), Some("web".into()));

    sub.handle(&proactive_event()).await;

    // Active channel is "web" — external channel should NOT be called.
    assert_eq!(state.send_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn skips_external_when_active_is_none() {
    let (ch, state) = channel("telegram", Some("telegram"));
    let map: HashMap<String, Arc<ChannelHandle>> = [("telegram".into(), ch)].into();
    let sub = ProactiveMessageSubscriber::new(Arc::new(map), None);

    sub.handle(&proactive_event()).await;

    assert_eq!(state.send_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn runtime_update_active_channel() {
    let (ch, state) = channel("discord", Some("discord"));
    let map: HashMap<String, Arc<ChannelHandle>> = [("discord".into(), ch)].into();
    let sub = ProactiveMessageSubscriber::new(Arc::new(map), None);

    // Initially no active channel — external not called.
    sub.handle(&proactive_event()).await;
    assert_eq!(state.send_count.load(Ordering::SeqCst), 0);

    // Update at runtime.
    sub.set_active_channel(Some("discord".into()));
    sub.handle(&proactive_event()).await;
    assert_eq!(state.send_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ignores_non_proactive_events() {
    let (ch, state) = channel("telegram", Some("telegram"));
    let map: HashMap<String, Arc<ChannelHandle>> = [("telegram".into(), ch)].into();
    let sub = ProactiveMessageSubscriber::new(Arc::new(map), Some("telegram".into()));

    sub.handle(&DomainEvent::CronJobTriggered {
        job_id: "j".into(),
        job_name: "test-job".into(),
        job_type: "agent".into(),
    })
    .await;

    assert_eq!(state.send_count.load(Ordering::SeqCst), 0);
}
