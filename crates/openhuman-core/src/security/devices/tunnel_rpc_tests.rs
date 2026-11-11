use super::*;

#[test]
fn tunnel_rpc_payload_requires_a_method_and_defaults_params() {
    let request = parse_tunnel_rpc_payload(serde_json::json!({"method": "devices_list"}))
        .expect("method-only payload is valid");

    assert_eq!(request.method, "devices_list");
    assert_eq!(request.params, Value::Null);
}

#[test]
fn tunnel_rpc_payload_rejects_missing_or_non_string_method() {
    for payload in [serde_json::json!({}), serde_json::json!({"method": 7})] {
        assert!(parse_tunnel_rpc_payload(payload).is_err());
    }
}

#[test]
fn tunnel_rpc_response_preserves_the_existing_envelope_shape() {
    let response = response_envelope("request-1", "response", serde_json::json!({"ok": true}));

    assert_eq!(
        serde_json::to_value(response).unwrap(),
        serde_json::json!({
            "requestId": "request-1",
            "kind": "response",
            "seq": 0,
            "payload": {"ok": true}
        })
    );
}

#[test]
fn tunnel_rpc_allows_only_user_surface_methods_and_ping() {
    for method in USER_METHODS.iter().copied().chain(["openhuman.ping"]) {
        assert!(
            is_tunnel_rpc_method_allowed(method),
            "{method} should be allowed"
        );
    }

    for method in [
        "devices.list",
        "openhuman.auth_set_credential",
        "openhuman.config_set",
        "openhuman.admin_shutdown",
        "openhuman.unknown_method",
    ] {
        assert!(
            !is_tunnel_rpc_method_allowed(method),
            "{method} should be denied"
        );
    }
}
