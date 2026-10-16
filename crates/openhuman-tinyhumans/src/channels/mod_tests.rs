use super::*;
use crate::hosted::test_support;
use serde_json::json;
use tinychannels_bus::module_progressive::{
    ProgressiveTransportRequest, ProgressiveTransportResult,
};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::client::HostedApiError;

fn ctx(
    config: openhuman_embed::__host::config::Config,
    channel: Option<&str>,
) -> ChannelCallbackCtx {
    ChannelCallbackCtx {
        config,
        channel: channel.map(str::to_string),
        session: CancellationToken::new(),
    }
}

fn typing(channel: &str) -> ChannelBackendRequest {
    ChannelBackendRequest::ProgressiveTransport(ProgressiveTransportRequest::Typing {
        channel: channel.to_string(),
    })
}

// Route absence and message absence need different recoveries in the
// progressive driver; conflating them once left a permanent "Thinking:"
// bubble in the chat (#5230). The mapping is by type, never by message text.
#[test]
fn backend_errors_map_to_distinct_progressive_recoveries() {
    let route_absent = anyhow::Error::new(HostedApiError::ChannelEditUnsupported {
        provider: "telegram".into(),
        message_id: "1103".into(),
    });
    assert!(matches!(
        delivery::map_backend_error(route_absent),
        ProgressiveTransportResult::EditUnsupported
    ));

    let message_absent = anyhow::Error::new(HostedApiError::MessageNotFound {
        provider: "telegram".into(),
        message_id: "1103".into(),
    });
    assert!(matches!(
        delivery::map_backend_error(message_absent),
        ProgressiveTransportResult::MessageGone
    ));

    // Anything untyped (5xx, transport, rate limit) keeps the retry budget.
    assert!(matches!(
        delivery::map_backend_error(anyhow::anyhow!("502 Bad Gateway")),
        ProgressiveTransportResult::Failed
    ));

    // A different typed variant must not be mistaken for either edit case.
    let unauthorized = anyhow::Error::new(HostedApiError::Unauthorized {
        method: "PATCH".into(),
        path: "/channels/telegram/messages/1103".into(),
    });
    assert!(matches!(
        delivery::map_backend_error(unauthorized),
        ProgressiveTransportResult::Failed
    ));
}

#[test]
fn registering_twice_leaves_one_callback() {
    let _registry = crate::registry_test_lock();

    register(&InstallOptions::default());
    register(&InstallOptions::default());
    assert!(openhuman_embed::__host::channel_callback::unregister_channel_callback(CALLBACK_ID));
}

#[tokio::test]
async fn typing_posts_to_the_channel_route_with_the_session_credential() {
    let tmp = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/telegram/typing"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"success": true, "data": {"ok": true}})),
        )
        .mount(&server)
        .await;
    let config = test_support::signed_in(&tmp, &server.uri());
    let answer = serve(ctx(config, Some("telegram")), typing("telegram"))
        .await
        .unwrap();
    match answer {
        ChannelBackendResult::ProgressiveTransport(ProgressiveTransportResult::Success(value)) => {
            assert_eq!(value, json!({"ok": true}));
        }
        _ => panic!("expected a transport success"),
    }
}

#[tokio::test]
async fn a_request_for_another_channel_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_support::signed_in(&tmp, "http://127.0.0.1:9");
    let answer = serve(ctx(config, Some("telegram")), typing("discord"))
        .await
        .unwrap();
    assert!(matches!(
        answer,
        ChannelBackendResult::ProgressiveTransport(ProgressiveTransportResult::Failed)
    ));
}

#[tokio::test]
async fn a_local_session_has_no_hosted_channel_credential() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_support::local_session(&tmp);
    let answer = serve(ctx(config, Some("telegram")), typing("telegram"))
        .await
        .unwrap();
    assert!(matches!(
        answer,
        ChannelBackendResult::ProgressiveTransport(ProgressiveTransportResult::Unavailable)
    ));
}

#[tokio::test]
async fn a_closed_session_is_unavailable_before_any_request() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_support::signed_in(&tmp, "http://127.0.0.1:9");
    let ctx = ctx(config, Some("telegram"));
    ctx.session.cancel();
    let answer = serve(ctx, typing("telegram")).await.unwrap();
    assert!(matches!(
        answer,
        ChannelBackendResult::ProgressiveTransport(ProgressiveTransportResult::Unavailable)
    ));
}

#[tokio::test]
async fn a_reaction_keeps_the_backend_payload_as_raw() {
    let tmp = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/telegram/reactions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"success": true, "data": {"unexpected": true}})),
        )
        .mount(&server)
        .await;
    let config = test_support::signed_in(&tmp, &server.uri());
    let answer = serve(
        ctx(config, Some("telegram")),
        ChannelBackendRequest::SendReaction {
            channel: "telegram".into(),
            reaction: json!({"emoji": "+1"}),
        },
    )
    .await
    .unwrap();
    match answer {
        ChannelBackendResult::SendReaction(result) => {
            assert_eq!(result.raw, Some(json!({"unexpected": true})));
        }
        _ => panic!("expected a reaction result"),
    }
}

#[tokio::test]
async fn thread_operations_use_the_thread_routes() {
    let tmp = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/telegram/threads"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"success": true, "data": {"id": "t1"}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/channels/telegram/threads"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"success": true, "data": {"odd": 1}})),
        )
        .mount(&server)
        .await;
    let config = test_support::signed_in(&tmp, &server.uri());
    let created = serve(
        ctx(config.clone(), None),
        ChannelBackendRequest::CreateThread {
            channel: "telegram".into(),
            title: "topic".into(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(created, ChannelBackendResult::Thread(_)));
    let listed = serve(
        ctx(config, None),
        ChannelBackendRequest::ListThreads {
            channel: "telegram".into(),
            active: None,
        },
    )
    .await
    .unwrap();
    match listed {
        ChannelBackendResult::Threads(result) => {
            assert_eq!(result.threads.len(), 0);
            assert_eq!(result.raw, Some(json!({"odd": 1})));
        }
        _ => panic!("expected a thread list"),
    }
}

#[tokio::test]
async fn requests_the_callback_does_not_serve_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_support::signed_in(&tmp, "http://127.0.0.1:9");
    let refused = serve(
        ctx(config, None),
        ChannelBackendRequest::Status { channel: None },
    )
    .await;
    assert!(refused.is_err());
}
