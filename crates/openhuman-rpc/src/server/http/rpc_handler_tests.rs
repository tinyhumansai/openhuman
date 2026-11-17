use serde_json::json;
#[cfg(feature = "crash-reporting")]
use std::sync::Arc;

use super::rpc_handler;
use crate::core_host::core::invoke::default_state;
use crate::server::testing::EnvVarGuard;

#[tokio::test(flavor = "current_thread")]
async fn structured_rpc_error_envelope_passes_through_generic_dispatch() {
    // The transport layer must surface any controller-emitted
    // `StructuredRpcError` payload without inspecting the method name —
    // this is what makes the boundary domain-agnostic. We register a
    // throwaway method-name on a thread-scoped op and confirm the
    // wire-shape carries the `kind`/`thread_id` data verbatim.
    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::Json;

    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let _env = EnvVarGuard::set_many(vec![(
        "OPENHUMAN_WORKSPACE",
        workspace.path().as_os_str().to_os_string(),
    )]);

    let stale_thread_request = crate::RpcRequest {
        jsonrpc: "2.0".to_string(),
        id: json!(7),
        method: "openhuman.threads_generate_title".to_string(),
        params: json!({ "thread_id": "thread-ghost" }),
    };
    let response = rpc_handler(State(default_state()), None, Json(stale_thread_request)).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json response");
    assert_eq!(body["error"]["data"]["kind"], "ThreadNotFound");
    assert_eq!(body["error"]["data"]["thread_id"], "thread-ghost");
    // The structured-error message must be human-readable on the wire —
    // never the encoded sentinel envelope.
    let message = body["error"]["message"].as_str().expect("error message");
    assert!(
        !message.contains("__OPENHUMAN_STRUCTURED_RPC_ERROR_V1__"),
        "sentinel-encoded envelope leaked onto the wire: {message}"
    );
    assert!(message.contains("thread-ghost"));
}

#[cfg(feature = "crash-reporting")]
#[tokio::test(flavor = "current_thread")]
async fn thread_not_found_rpc_error_does_not_report_to_sentry() {
    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::Json;
    use sentry::test::TestTransport;
    use tracing::Level;
    use tracing_subscriber::layer::SubscriberExt;

    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let _env = EnvVarGuard::set_many(vec![(
        "OPENHUMAN_WORKSPACE",
        workspace.path().as_os_str().to_os_string(),
    )]);

    let transport = TestTransport::new();
    let sentry_options = sentry::ClientOptions {
        dsn: Some("https://public@sentry.invalid/1".parse().unwrap()),
        transport: Some(Arc::new(transport.clone())),
        ..Default::default()
    };
    let sentry_hub = Arc::new(sentry::Hub::new(
        Some(Arc::new(sentry_options.into())),
        Arc::new(Default::default()),
    ));
    let _sentry_guard = sentry::HubSwitchGuard::new(sentry_hub);

    let subscriber = tracing_subscriber::registry().with(
        sentry::integrations::tracing::layer().event_filter(|metadata| {
            // Mirror the production sentry-tracing layer: events emitted from
            // `report_error_message` are captured directly via
            // `sentry::capture_message` and must not be picked up here too
            // (otherwise this test sees double events).
            if metadata.target()
                == crate::core_host::core::observability::REPORT_ERROR_TRACING_TARGET
            {
                return sentry::integrations::tracing::EventFilter::Ignore;
            }
            match *metadata.level() {
                Level::ERROR => sentry::integrations::tracing::EventFilter::Event,
                Level::WARN | Level::INFO => sentry::integrations::tracing::EventFilter::Breadcrumb,
                _ => sentry::integrations::tracing::EventFilter::Ignore,
            }
        }),
    );
    let _subscriber_guard = tracing::subscriber::set_default(subscriber);

    let stale_thread_request = crate::RpcRequest {
        jsonrpc: "2.0".to_string(),
        id: json!(1),
        method: "openhuman.threads_message_append".to_string(),
        params: json!({
            "thread_id": "thread-missing",
            "message": {
                "id": "msg-1",
                "content": "hello",
                "type": "text",
                "extraMetadata": {},
                "sender": "user",
                "createdAt": "2026-01-01T00:00:00Z"
            }
        }),
    };
    let response = rpc_handler(State(default_state()), None, Json(stale_thread_request)).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json response");
    assert_eq!(body["error"]["data"]["kind"], "ThreadNotFound");
    assert!(
        transport.fetch_and_clear_events().is_empty(),
        "ThreadNotFound should not reach Sentry"
    );

    let unrelated_error_request = crate::RpcRequest {
        jsonrpc: "2.0".to_string(),
        id: json!(2),
        method: "core.not_a_real_method".to_string(),
        params: json!({}),
    };
    let response = rpc_handler(State(default_state()), None, Json(unrelated_error_request)).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json response");
    assert_eq!(body["error"]["data"], serde_json::Value::Null);

    let events = transport.fetch_and_clear_events();
    assert_eq!(
        events.len(),
        1,
        "unrelated RPC errors should still reach Sentry"
    );
    assert_eq!(
        events[0].tags.get("domain").map(String::as_str),
        Some("rpc")
    );
    assert_eq!(
        events[0].tags.get("operation").map(String::as_str),
        Some("invoke_method")
    );
    assert_eq!(
        events[0].tags.get("method").map(String::as_str),
        Some("core.not_a_real_method")
    );
    // #3567: an unrecognised (non-allow-listed) method is still recorded for
    // triage, but downgraded from error to *warning* severity so it no longer
    // pages. The JSON-RPC method-not-found response above is unchanged.
    assert_eq!(
        events[0].level,
        sentry::Level::Warning,
        "unknown-method events should be warn-level (triage, not paging)"
    );
}

#[cfg(feature = "crash-reporting")]
#[tokio::test(flavor = "current_thread")]
async fn unknown_method_severity_split_by_probe_allow_list() {
    // #3567: prove the full severity split at the transport boundary —
    // (1) an allow-listed probe name is NOT captured to Sentry (debug-only),
    // (2) a genuinely-unknown method still surfaces at warn for triage,
    // (3) the JSON-RPC error response to the caller is unchanged in both cases.
    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::Json;
    use sentry::test::TestTransport;
    use tracing::Level;
    use tracing_subscriber::layer::SubscriberExt;

    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let _env = EnvVarGuard::set_many(vec![(
        "OPENHUMAN_WORKSPACE",
        workspace.path().as_os_str().to_os_string(),
    )]);

    let transport = TestTransport::new();
    let sentry_options = sentry::ClientOptions {
        dsn: Some("https://public@sentry.invalid/1".parse().unwrap()),
        transport: Some(Arc::new(transport.clone())),
        ..Default::default()
    };
    let sentry_hub = Arc::new(sentry::Hub::new(
        Some(Arc::new(sentry_options.into())),
        Arc::new(Default::default()),
    ));
    let _sentry_guard = sentry::HubSwitchGuard::new(sentry_hub);

    let subscriber = tracing_subscriber::registry().with(
        sentry::integrations::tracing::layer().event_filter(|metadata| {
            // Mirror production: diagnostics from the report_* helpers are
            // captured directly via `sentry::capture_message`, so the bridge
            // must ignore their marker target to avoid double events.
            if metadata.target()
                == crate::core_host::core::observability::REPORT_ERROR_TRACING_TARGET
            {
                return sentry::integrations::tracing::EventFilter::Ignore;
            }
            match *metadata.level() {
                Level::ERROR => sentry::integrations::tracing::EventFilter::Event,
                Level::WARN | Level::INFO => sentry::integrations::tracing::EventFilter::Breadcrumb,
                _ => sentry::integrations::tracing::EventFilter::Ignore,
            }
        }),
    );
    let _subscriber_guard = tracing::subscriber::set_default(subscriber);

    // (1) Allow-listed probe → debug-only, never reaches Sentry.
    let probe_request = crate::RpcRequest {
        jsonrpc: "2.0".to_string(),
        id: json!(1),
        method: "rpc.discover".to_string(),
        params: json!({}),
    };
    let response = rpc_handler(State(default_state()), None, Json(probe_request)).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json response");
    // (3) Response is the unchanged JSON-RPC method-not-found envelope.
    assert_eq!(body["error"]["code"], json!(-32000));
    assert_eq!(
        body["error"]["message"],
        json!("unknown method: rpc.discover")
    );
    assert_eq!(body["error"]["data"], serde_json::Value::Null);
    assert!(
        transport.fetch_and_clear_events().is_empty(),
        "allow-listed probe methods must not reach Sentry"
    );

    // (2) Genuinely-unknown method → still captured, but at warn for triage.
    let unknown_request = crate::RpcRequest {
        jsonrpc: "2.0".to_string(),
        id: json!(2),
        method: "totally.made.up.method".to_string(),
        params: json!({}),
    };
    let response = rpc_handler(State(default_state()), None, Json(unknown_request)).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json response");
    // (3) Same unchanged method-not-found envelope for the unknown method.
    assert_eq!(body["error"]["code"], json!(-32000));
    assert_eq!(
        body["error"]["message"],
        json!("unknown method: totally.made.up.method")
    );
    assert_eq!(body["error"]["data"], serde_json::Value::Null);

    let events = transport.fetch_and_clear_events();
    assert_eq!(
        events.len(),
        1,
        "genuinely-unknown methods should still be captured for triage"
    );
    assert_eq!(events[0].level, sentry::Level::Warning);
    assert_eq!(
        events[0].tags.get("domain").map(String::as_str),
        Some("rpc")
    );
    assert_eq!(
        events[0].tags.get("method").map(String::as_str),
        Some("totally.made.up.method")
    );
}

#[cfg(feature = "crash-reporting")]
#[tokio::test(flavor = "current_thread")]
async fn invalid_memory_ingest_is_not_reported_to_sentry() {
    // #5169 (CORE-RUST-1P0): a caller submitting an ingest payload that does
    // not match the canonicaliser schema is a *caller* error — the handler
    // already names the offending field and no core change can fix a producer
    // sending the wrong shape. Prove the same split as the unknown-method case
    // above: the JSON-RPC error response is unchanged, but the Sentry event is
    // warn (triage) rather than error (pages).
    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::Json;
    use sentry::test::TestTransport;
    use tracing::Level;
    use tracing_subscriber::layer::SubscriberExt;

    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let _env = EnvVarGuard::set_many(vec![(
        "OPENHUMAN_WORKSPACE",
        workspace.path().as_os_str().to_os_string(),
    )]);

    let transport = TestTransport::new();
    let sentry_options = sentry::ClientOptions {
        dsn: Some("https://public@sentry.invalid/1".parse().unwrap()),
        transport: Some(Arc::new(transport.clone())),
        ..Default::default()
    };
    let sentry_hub = Arc::new(sentry::Hub::new(
        Some(Arc::new(sentry_options.into())),
        Arc::new(Default::default()),
    ));
    let _sentry_guard = sentry::HubSwitchGuard::new(sentry_hub);

    let subscriber = tracing_subscriber::registry().with(
        sentry::integrations::tracing::layer().event_filter(|metadata| {
            if metadata.target()
                == crate::core_host::core::observability::REPORT_ERROR_TRACING_TARGET
            {
                return sentry::integrations::tracing::EventFilter::Ignore;
            }
            match *metadata.level() {
                Level::ERROR => sentry::integrations::tracing::EventFilter::Event,
                Level::WARN | Level::INFO => sentry::integrations::tracing::EventFilter::Breadcrumb,
                _ => sentry::integrations::tracing::EventFilter::Ignore,
            }
        }),
    );
    let _subscriber_guard = tracing::subscriber::set_default(subscriber);

    // A missing file is an expected input error for the current memory v2
    // ingest endpoint, so it should be returned to the caller without a
    // Sentry event.
    let request = crate::RpcRequest {
        jsonrpc: "2.0".to_string(),
        id: json!(1),
        method: "openhuman.memory_brain_ingest".to_string(),
        params: json!({ "path": "/openhuman-test/missing-memory-ingest-file" }),
    };
    let response = rpc_handler(State(default_state()), None, Json(request)).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json response");

    // The caller still gets the validation error.
    assert_eq!(body["error"]["code"], json!(-32000));
    let message = body["error"]["message"]
        .as_str()
        .expect("error message string");
    assert!(
        message.contains("cannot read the file:"),
        "expected a missing-file validation error, got {message:?}"
    );

    let events = transport.fetch_and_clear_events();
    assert!(
        events.is_empty(),
        "expected user input errors should not be reported to Sentry"
    );
}
