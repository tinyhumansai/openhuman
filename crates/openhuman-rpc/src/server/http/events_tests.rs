use axum::body::to_bytes;
use axum::extract::Query;
use axum::http::{header, HeaderMap, StatusCode};

use super::{
    active_workspace_handle, domain_event_payload, domain_event_stream_unavailable,
    domain_events_handler, events_handler, webhook_events_handler, EventsQuery,
};

#[test]
fn domain_event_stream_status_requires_enabled_config_and_initialized_bus() {
    assert_eq!(
        domain_event_stream_unavailable(false, false)
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        domain_event_stream_unavailable(true, false)
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(domain_event_stream_unavailable(true, true).is_none());
}

#[test]
fn active_workspace_resolution_failure_is_nonfatal() {
    let result = active_workspace_handle(Err(anyhow::anyhow!("workspace unavailable")));
    assert!(result.is_none());
}

#[test]
fn domain_event_payload_includes_redacted_detail_and_workspace_handle() {
    let event = crate::core_host::core::events::DomainEvent::McpServerProbeTimedOut {
        server_id: "server-1".into(),
        qualified_name: "example.test/mcp".into(),
        probe_timeout_secs: 10,
        consecutive_timeouts: 2,
        teardown_after: 3,
        workspace_dir: std::path::PathBuf::from("/home/private/workspace"),
    };

    let (domain, data) = domain_event_payload(&event).expect("event serializes");
    let data: serde_json::Value = serde_json::from_str(&data).expect("valid event JSON");

    assert_eq!(domain, "mcp_client");
    assert_eq!(data["event"], "McpServerProbeTimedOut");
    assert_eq!(data["agent"], "example.test/mcp");
    assert_eq!(
        data["detail"],
        "no answer in 10s; timeout 2 of 3 before teardown"
    );
    assert_ne!(data["workspace"], "/home/private/workspace");
    assert!(data["timestamp"].as_str().is_some());
}

fn query(client_id: &str, token: Option<&str>) -> Query<EventsQuery> {
    Query(EventsQuery {
        client_id: client_id.to_string(),
        token: token.map(str::to_string),
    })
}

#[tokio::test]
async fn events_require_a_credential_and_reject_unknown_bind_tokens() {
    let missing = events_handler(HeaderMap::new(), None, query("client", None)).await;
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(missing.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("Missing credentials"));

    let invalid = events_handler(HeaderMap::new(), None, query("client", Some("unknown"))).await;
    assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(invalid.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("unknown, expired"));
}

#[tokio::test]
async fn events_bind_token_is_client_bound_and_single_use() {
    let token = crate::core_host::core::event_bind_tokens::issue("right", None)
        .expect("bind token")
        .token;

    let wrong = events_handler(HeaderMap::new(), None, query("wrong", Some(&token))).await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    let accepted = events_handler(HeaderMap::new(), None, query("right", Some(&token))).await;
    assert_eq!(accepted.status(), StatusCode::OK);
    assert_eq!(
        accepted.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    drop(accepted);

    let replay = events_handler(HeaderMap::new(), None, query("right", Some(&token))).await;
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn events_stream_forwards_only_the_bound_client() {
    use crate::core_host::web_chat::{publish_web_channel_event, WebChannelEvent};
    use tokio_stream::StreamExt;

    let token = crate::core_host::core::event_bind_tokens::issue("stream-client", None)
        .expect("bind token")
        .token;
    let response =
        events_handler(HeaderMap::new(), None, query("stream-client", Some(&token))).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();

    publish_web_channel_event(WebChannelEvent::UserError {
        context: crate::core_host::web_chat::WebEventContext {
            client_id: "another-client".into(),
            ..Default::default()
        },
        error_type: Some("wrong_client_probe".into()),
        error_source: None,
    });
    publish_web_channel_event(WebChannelEvent::UserError {
        context: crate::core_host::web_chat::WebEventContext {
            client_id: "stream-client".into(),
            ..Default::default()
        },
        error_type: Some("right_client_probe".into()),
        error_source: None,
    });

    let chunk = tokio::time::timeout(std::time::Duration::from_secs(1), body.next())
        .await
        .expect("matching event arrived")
        .expect("SSE body chunk")
        .expect("SSE bytes");
    let event = String::from_utf8_lossy(&chunk);
    assert!(event.contains("event: user_error"));
    assert!(event.contains("right_client_probe"));
    assert!(!event.contains("wrong_client_probe"));
}

#[tokio::test]
async fn domain_events_require_a_bearer() {
    let response = domain_events_handler(HeaderMap::new(), None).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("Bearer token required"));
}

#[tokio::test(flavor = "current_thread")]
async fn domain_events_stream_config_then_published_events() {
    use crate::core_host::core::events::DomainEvent;
    use futures::StreamExt;
    use std::ffi::OsString;

    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let _env = crate::server::testing::EnvVarGuard::set_many(vec![(
        "OPENHUMAN_WORKSPACE",
        OsString::from(workspace.path()),
    )]);
    crate::core_host::core::auth::init_rpc_token_with_value("events-http-tests-token")
        .expect("initialize test bearer");
    let token = crate::core_host::core::auth::get_rpc_token()
        .expect("test bearer initialized")
        .to_string();
    if crate::core_host::core::bus::BUS.get().is_none() {
        crate::core_host::core::bus::init()
            .await
            .expect("initialize in-process event bus");
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    let response = domain_events_handler(headers, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut chunks = response.into_body().into_data_stream();

    let config = tokio::time::timeout(std::time::Duration::from_secs(1), chunks.next())
        .await
        .expect("config event arrives")
        .expect("SSE config chunk")
        .expect("SSE bytes");
    assert!(String::from_utf8_lossy(&config).contains("event: config"));

    crate::core_host::core::bus::BUS.publish(DomainEvent::SystemStartup {
        component: "events-test".into(),
    });
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), chunks.next())
        .await
        .expect("published event arrives")
        .expect("SSE domain event chunk")
        .expect("SSE bytes");
    let event = String::from_utf8_lossy(&event);
    assert!(event.contains("event: system"));
    assert!(event.contains("SystemStartup"));
}

#[tokio::test]
async fn webhook_debug_stream_starts_with_a_documented_event() {
    let response = webhook_events_handler().await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    let mut body = response.into_body().into_data_stream();
    use tokio_stream::StreamExt;
    let chunk = body.next().await.expect("first event").expect("SSE bytes");
    let event = String::from_utf8_lossy(&chunk);
    assert!(event.contains("event: webhooks_debug"));
    assert!(event.contains("runtime_removed"));
}
