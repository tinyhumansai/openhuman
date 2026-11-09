use super::*;
use crate::core::all::{
    assert_schema_controller_parity, verify_parameter_schema, ControllerHandler,
    ControllerParamContract,
};
use crate::core::runtime::DispatchCtx;

#[test]
fn socket_schema_contracts_are_preserved() {
    let schemas = all_controller_schemas();
    crate::core::all::expect_schema_namespace(&schemas, "socket");
    assert_schema_controller_parity(&schemas, &all_registered_controllers(), "connect");
    let contracts = socket_parameter_contracts();
    for contract in &contracts {
        let schema = schemas
            .iter()
            .find(|schema| schema.method_name() == contract.method)
            .expect("socket schema has an independent contract");
        verify_parameter_schema(schema, contract)
            .expect("independent socket schema contract matches");
    }
    for schema in schemas {
        assert!(
            !schema.outputs.is_empty(),
            "{} output fields",
            schema.function
        );
    }
}

pub(super) fn socket_parameter_contracts() -> Vec<ControllerParamContract> {
    use serde_json::json;

    vec![
        ControllerParamContract {
            method: "socket.connect",
            required: &["url", "token"],
            optional: &[],
            examples: vec![
                json!({"url":"https://example.invalid/socket", "token":"test-token"}),
                json!({"url":"", "token":""}),
            ],
        },
        ControllerParamContract {
            method: "socket.disconnect",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
        ControllerParamContract {
            method: "socket.state",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
        ControllerParamContract {
            method: "socket.emit",
            required: &["event"],
            optional: &["data"],
            examples: vec![
                json!({"event":"message", "data":{"text":"hello"}}),
                json!({"event":"message"}),
            ],
        },
        ControllerParamContract {
            method: "socket.connect_with_session",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
    ]
}

#[test]
fn socket_legacy_catalog_keeps_handler_identities() {
    let expected: [(&str, ControllerHandler); 5] = [
        ("socket.connect", handle_connect),
        ("socket.disconnect", handle_disconnect),
        ("socket.state", handle_state),
        ("socket.emit", handle_emit),
        ("socket.connect_with_session", handle_connect_with_session),
    ];
    let registered = all_registered_controllers();
    assert_eq!(registered.len(), expected.len());
    for ((method, handler), controller) in expected.iter().zip(&registered) {
        assert_eq!(controller.schema.method_name(), *method);
        assert!(std::ptr::fn_addr_eq(controller.handler, *handler));
    }
}

#[test]
fn connect_parser_keeps_reader_order_and_string_values() {
    let error = parse_connect(Map::new()).err().expect("missing URL fails");
    assert_eq!(error, "missing required param 'url'");

    let params = serde_json::from_value(serde_json::json!({"url":" ", "extra":true}))
        .expect("object params");
    assert_eq!(
        parse_connect(params).err().expect("missing token fails"),
        "missing required param 'token'"
    );

    let params = serde_json::from_value(serde_json::json!({
        "url":"  https://example.invalid/socket  ",
        "token":" token ",
        "unknown":{"ignored":true}
    }))
    .expect("object params");
    let parsed = parse_connect(params).expect("string values remain permissive");
    assert_eq!(parsed.url, "  https://example.invalid/socket  ");
    assert_eq!(parsed.token, " token ");

    for invalid in [
        serde_json::json!({"url":null,"token":"token"}),
        serde_json::json!({"url":42,"token":"token"}),
    ] {
        let params = serde_json::from_value(invalid).expect("object params");
        assert_eq!(
            parse_connect(params)
                .err()
                .expect("invalid URL is rejected before token"),
            "missing required param 'url'"
        );
    }
    for invalid_token in [
        serde_json::Value::Null,
        serde_json::json!(42),
        serde_json::json!({"token":"nested"}),
    ] {
        let params = serde_json::json!({"url":"url", "token":invalid_token});
        assert_eq!(
            parse_connect(serde_json::from_value(params).expect("object params"))
                .err()
                .expect("non-string token is rejected"),
            "missing required param 'token'"
        );
    }
}

#[test]
fn emit_parser_defaults_data_to_null_and_preserves_any_json_value() {
    let missing = serde_json::from_value(serde_json::json!({"event":" message ", "extra":1}))
        .expect("object params");
    let parsed = parse_emit(missing).expect("event is a string");
    assert_eq!(parsed.event, " message ");
    assert_eq!(parsed.data, Value::Null);

    for data in [
        serde_json::Value::Null,
        serde_json::json!(42),
        serde_json::json!("payload"),
        serde_json::json!([true, null]),
        serde_json::json!({"nested":{"value":true}}),
    ] {
        let params = serde_json::json!({"event":"message", "data":data.clone()});
        let parsed = parse_emit(serde_json::from_value(params).expect("object params"))
            .expect("all JSON payload values remain accepted");
        assert_eq!(parsed.data, data);
    }
    for params in [
        serde_json::json!({}),
        serde_json::json!({"event":null}),
        serde_json::json!({"event":42}),
    ] {
        assert_eq!(
            parse_emit(serde_json::from_value(params).expect("object params"))
                .err()
                .expect("missing or non-string event fails"),
            "missing required param 'event'"
        );
    }
}

#[test]
fn ignored_map_parser_accepts_unknown_values() {
    let params: Map<String, Value> = serde_json::from_value(serde_json::json!({
        "anything":[null, true, {"nested":42}],
        "other":"ignored"
    }))
    .expect("object params");
    assert_eq!(parse_ignored_params(params.clone()), Ok(()));
    let entries = all_controller_entries();
    for method in [
        "socket.disconnect",
        "socket.state",
        "socket.connect_with_session",
    ] {
        entries
            .iter()
            .find(|entry| entry.schema.method_name() == method)
            .expect("ignored-map method has a bound registry entry")
            .decode_params(params.clone())
            .expect("actual parser deliberately accepts unknown keys and values");
    }
}

// ── handlers (without manager): require_manager errors ─────────

#[tokio::test]
async fn handlers_error_without_initialized_manager() {
    // Production bootstrap calls `set_global_socket_manager` once; in
    // these unit tests the global singleton is intentionally NOT set,
    // so every handler should hit the `SocketManager not initialized`
    // branch via `require_manager()` first.
    //
    // We can't reliably clear a OnceLock once set. If another test in
    // the same binary has already installed a global manager, skip
    // rather than cross-contaminating.
    if super::global_socket_manager().is_some() {
        eprintln!(
            "[socket:schemas tests] global manager already installed — \
             skipping require_manager error-path assertions"
        );
        return;
    }

    let err = handle_disconnect(DispatchCtx::process_default(), Map::new())
        .await
        .unwrap_err();
    assert!(err.contains("SocketManager not initialized"));

    let err = handle_state(DispatchCtx::process_default(), Map::new())
        .await
        .unwrap_err();
    assert!(err.contains("SocketManager not initialized"));

    let err = handle_connect(DispatchCtx::process_default(), Map::new())
        .await
        .unwrap_err();
    assert!(err.contains("SocketManager not initialized"));

    let err = handle_emit(DispatchCtx::process_default(), Map::new())
        .await
        .unwrap_err();
    assert!(err.contains("SocketManager not initialized"));

    let err = handle_connect_with_session(DispatchCtx::process_default(), Map::new())
        .await
        .unwrap_err();
    assert!(err.contains("SocketManager not initialized"));
}

/// #6111 — the lifecycle handlers and `socket_state` must publish `status` in one vocabulary.
///
/// The three lifecycle handlers used to build their payload with `format!("{:?}", status)` while
/// `socket_state` went through `ConnectionStatus`' `#[serde(rename_all = "lowercase")]`, so one
/// namespace answered `"Disconnected"` and `"disconnected"` for the same field. This pins the
/// decided direction (serde) against the source of truth — the serde encoding itself — rather
/// than against a hand-written list that could drift with it.
#[test]
fn status_payload_matches_the_serde_encoding_for_every_variant() {
    for status in [
        ConnectionStatus::Disconnected,
        ConnectionStatus::Connecting,
        ConnectionStatus::Connected,
        ConnectionStatus::Reconnecting,
        ConnectionStatus::Error,
    ] {
        let state = SocketState {
            status,
            socket_id: None,
            error: None,
        };

        // What `socket_state` publishes for this status.
        let from_state = serde_json::to_value(&state).expect("serialize SocketState");
        let expected = from_state
            .get("status")
            .and_then(Value::as_str)
            .expect("SocketState serialises a status field");

        // What the lifecycle handlers publish for the same status.
        let payload = status_payload(&state);
        assert_eq!(
            payload.get("status").and_then(Value::as_str),
            Some(expected),
            "lifecycle handlers and socket_state must agree on {status:?}"
        );
        assert_ne!(
            payload.get("status").and_then(Value::as_str),
            Some(format!("{status:?}").as_str()),
            "{status:?} must not be published in Rust's Debug spelling"
        );
    }
}

/// The slug is lowercase for every variant — the property `connectivity_diag` and the frontend's
/// lowercase comparisons rely on.
#[test]
fn status_slug_is_lowercase_for_every_variant() {
    for status in [
        ConnectionStatus::Disconnected,
        ConnectionStatus::Connecting,
        ConnectionStatus::Connected,
        ConnectionStatus::Reconnecting,
        ConnectionStatus::Error,
    ] {
        let slug = status_slug(status);
        assert!(!slug.is_empty(), "{status:?} produced an empty slug");
        assert_eq!(
            slug,
            slug.to_lowercase(),
            "{status:?} slug is not lowercase"
        );
    }
}
