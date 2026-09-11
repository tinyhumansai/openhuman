use super::*;

fn unique_component(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

#[tokio::test]
async fn health_changed_false_records_error() {
    let component = unique_component("health-bus-error");
    let sub = HealthSubscriber;
    sub.handle(&DomainEvent::HealthChanged {
        component: component.clone(),
        healthy: false,
        message: Some("boom".into()),
    })
    .await;

    let snapshot = crate::openhuman::platform::health::snapshot();
    let entry = snapshot.components.get(&component).unwrap();
    assert_eq!(entry.status, "error");
    assert_eq!(entry.last_error.as_deref(), Some("boom"));
}

#[tokio::test]
async fn channel_disconnected_marks_channel_component_error() {
    let channel = format!("health-bus-channel-{}", uuid::Uuid::new_v4());
    let sub = HealthSubscriber;
    sub.handle(&DomainEvent::ChannelDisconnected {
        channel: channel.clone(),
        reason: "offline".into(),
    })
    .await;

    let snapshot = crate::openhuman::platform::health::snapshot();
    let entry = snapshot
        .components
        .get(&format!("channel:{channel}"))
        .unwrap();
    assert_eq!(entry.status, "error");
}
