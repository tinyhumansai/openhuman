use super::*;
use tinyflows_bus::schedule::types::DeliveryStatus;

#[tokio::test]
async fn announce_to_unavailable_channel_is_reported_as_failed() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let missing_channel = format!("missing-{}", uuid::Uuid::new_v4());
    let mut job = test_job("echo ok");
    job.delivery = DeliveryConfig {
        mode: "announce".into(),
        channel: Some(missing_channel),
        to: Some("target".into()),
        best_effort: false,
    };

    let result = deliver_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &job,
        "run-1",
        "hello",
        true,
    )
    .await;

    assert_eq!(
        result.unwrap(),
        DeliveryStatus::Failed,
        "an announce without a registered channel must not report Delivered"
    );
}

/// An announce to a configured account goes through the channel module's
/// progressive output and reaches the provider; the run's id is the turn id.
#[tokio::test]
async fn announce_success_is_delivered_through_the_channel_module_to_the_provider() {
    let linked = crate::channels::runtime::test_support::linked_telegram_runtime().await;
    crate::cron::channel_bridge::register_channel_bridge(linked.module_objects());
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let mut job = test_job("echo ok");
    job.delivery = DeliveryConfig {
        mode: "announce".into(),
        channel: Some("telegram".into()),
        to: Some("42".into()),
        best_effort: false,
    };

    let result = deliver_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &job,
        "run-announce-success",
        "announced hello",
        true,
    )
    .await;
    crate::cron::channel_bridge::clear_channel_bridge();

    assert_eq!(result.unwrap(), DeliveryStatus::Delivered);
    let requests = linked.provider_request_paths().await;
    assert!(
        requests.iter().any(|path| path.ends_with("/sendMessage")),
        "the provider should receive the announcement: {requests:?}"
    );
}
