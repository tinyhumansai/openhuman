use super::*;
use crate::channels::controllers::OpenHumanChannelBackend;
use tinychannels_bus::module_progressive::{
    ProgressiveTransportRequest, ProgressiveTransportResult,
};

fn typing(channel: &str) -> ChannelBackendRequest {
    ChannelBackendRequest::ProgressiveTransport(ProgressiveTransportRequest::Typing {
        channel: channel.to_string(),
    })
}

fn answering(
    value: serde_json::Value,
) -> impl Fn(
    ChannelCallbackCtx,
    ChannelBackendRequest,
) -> CallbackFuture<anyhow::Result<ChannelBackendResult>> {
    move |_, _| {
        let value = value.clone();
        Box::pin(async move {
            Ok(ChannelBackendResult::ProgressiveTransport(
                ProgressiveTransportResult::Success(value),
            ))
        })
    }
}

#[test]
fn nothing_registered_has_no_callback() {
    assert!(current().is_none());
}

#[test]
fn the_first_registered_callback_serves_requests() {
    register_channel_callback("first", answering(serde_json::json!(1)));
    register_channel_callback("second", answering(serde_json::json!(2)));
    assert!(current().is_some());
    assert!(unregister_channel_callback("first"));
    assert!(current().is_some(), "the second callback takes over");
    assert!(unregister_channel_callback("second"));
    assert!(current().is_none());
}

#[tokio::test]
async fn a_registered_callback_serves_the_progressive_transport_before_the_refusal() {
    let backend = OpenHumanChannelBackend::new(Config::default());
    let refused = backend.dispatch_callback(typing("web")).await;
    assert!(
        refused.is_err(),
        "without a callback the request is refused"
    );

    register_channel_callback("test", answering(serde_json::json!({"ok": true})));
    let served = backend.dispatch_callback(typing("web")).await;
    unregister_channel_callback("test");

    match served.expect("callback answers") {
        ChannelBackendResult::ProgressiveTransport(ProgressiveTransportResult::Success(value)) => {
            assert_eq!(value, serde_json::json!({"ok": true}));
        }
        _ => panic!("expected the registered callback's result"),
    }
}

#[tokio::test]
async fn the_callback_is_told_the_config_it_serves() {
    let seen = Arc::new(std::sync::Mutex::new(None));
    let sink = Arc::clone(&seen);
    register_channel_callback("ctx", move |ctx, _| {
        *sink.lock().unwrap() = Some(ctx.config.api_url.clone());
        Box::pin(async {
            Ok(ChannelBackendResult::ProgressiveTransport(
                ProgressiveTransportResult::Failed,
            ))
        })
    });
    let config = Config {
        api_url: Some("https://callback.example".to_string()),
        ..Config::default()
    };
    let _ = OpenHumanChannelBackend::new(config)
        .dispatch_callback(typing("web"))
        .await;
    unregister_channel_callback("ctx");
    assert_eq!(
        seen.lock().unwrap().clone().flatten().as_deref(),
        Some("https://callback.example")
    );
}

#[tokio::test]
async fn reactions_and_threads_without_a_callback_answer_the_unavailable_sentinel() {
    let backend = OpenHumanChannelBackend::new(Config::default());
    let channels = crate::config::ChannelsConfig::default();
    let sentinel = crate::core::observability::BACKEND_UNAVAILABLE_PREFIX;
    let reaction = backend
        .send_reaction(&channels, "telegram", serde_json::json!({"emoji": "+1"}))
        .await
        .err()
        .expect("no callback");
    assert!(reaction.to_string().starts_with(sentinel), "{reaction}");
    let listed = backend
        .list_threads(&channels, "telegram", None)
        .await
        .err()
        .expect("no callback");
    assert!(listed.to_string().starts_with(sentinel), "{listed}");
}

#[tokio::test]
async fn a_registered_callback_serves_thread_requests_with_the_channel_it_is_told() {
    use tinychannels_bus::controllers::ChannelThreadResult;
    let seen = Arc::new(std::sync::Mutex::new(None));
    let sink = Arc::clone(&seen);
    register_channel_callback("threads", move |ctx, request| {
        *sink.lock().unwrap() = Some((
            ctx.channel.clone(),
            matches!(request, ChannelBackendRequest::CreateThread { .. }),
        ));
        Box::pin(async {
            Ok(ChannelBackendResult::Thread(ChannelThreadResult {
                raw: Some(serde_json::json!({"id": "t1"})),
                ..Default::default()
            }))
        })
    });
    let backend = OpenHumanChannelBackend::new(Config::default());
    let created = backend
        .create_thread(
            &crate::config::ChannelsConfig::default(),
            "telegram",
            "topic",
        )
        .await;
    unregister_channel_callback("threads");
    assert_eq!(
        created.expect("served").raw,
        Some(serde_json::json!({"id": "t1"}))
    );
    assert_eq!(
        seen.lock().unwrap().clone(),
        Some((Some("telegram".to_string()), true))
    );
}
