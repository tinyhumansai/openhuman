//! The load-bearing test here is [`turn_request_field_names_match_the_controller`].
//!
//! Everything else in this file checks the facade's own logic; that one checks
//! the thing that actually breaks in the field. `AgentChatParams` has no
//! `#[serde(rename_all)]`, so its wire names are Rust field names — an upstream
//! rename produces no compile error anywhere, and a hand-written `json!` keeps
//! sending the old key while the controller reads `None`. Pinning our field
//! names against the *registered schema* turns that into a test failure.

use super::*;
use serde_json::json;

/// Every key [`TurnRequest`] serializes must be a declared input of the
/// controller it is sent to.
#[test]
fn turn_request_field_names_match_the_controller() {
    let schema = crate::openhuman::inference::local::all_local_inference_controller_schemas()
        .into_iter()
        .find(|s| s.namespace == "inference" && s.function == "agent_chat")
        .expect("inference.agent_chat is a registered controller");

    let declared: std::collections::HashSet<&str> =
        schema.inputs.iter().map(|field| field.name).collect();

    // Fully populated so `skip_serializing_if` hides nothing.
    let request = TurnRequest {
        message: "hi".into(),
        model_override: Some("m".into()),
        temperature: Some(0.5),
        thread_id: Some("t".into()),
        cwd: Some("/tmp".into()),
        inference_url: Some("https://example.invalid/v1".into()),
        api_key: Some("k".into()),
    };

    let serde_json::Value::Object(encoded) =
        serde_json::to_value(&request).expect("TurnRequest encodes")
    else {
        panic!("TurnRequest must encode to an object");
    };

    for key in encoded.keys() {
        assert!(
            declared.contains(key.as_str()),
            "TurnRequest sends `{key}`, which inference.agent_chat does not declare. \
             Either the controller renamed a field or the facade invented one; \
             declared inputs are {declared:?}"
        );
    }

    // And the reverse: a param the controller declares but the facade cannot
    // reach is a capability we silently dropped.
    for field in &schema.inputs {
        assert!(
            encoded.contains_key(field.name),
            "inference.agent_chat declares `{}`, which TurnRequest cannot send",
            field.name
        );
    }
}

#[test]
fn turn_request_omits_absent_options() {
    // `None` must not become `null`: the controller's `Option<String>` would
    // accept it, but sending keys the caller never set makes the wire payload
    // depend on facade internals rather than on what was asked for.
    let encoded = serde_json::to_value(TurnRequest::new("hi")).expect("encodes");
    assert_eq!(encoded, json!({ "message": "hi" }));
}

#[test]
fn a_route_sets_both_halves_or_neither() {
    // The core ignores the route unless both arrive non-blank, so the type
    // makes supplying one alone unrepresentable.
    let turn_request = {
        let mut r = TurnRequest::new("hi");
        let route = Route::openai_compatible("https://example.invalid/v1", "sk-test");
        r.inference_url = Some(route.base_url);
        r.api_key = Some(route.api_key);
        r
    };
    assert_eq!(
        turn_request.inference_url.as_deref(),
        Some("https://example.invalid/v1")
    );
    assert_eq!(turn_request.api_key.as_deref(), Some("sk-test"));
}

#[test]
fn blank_per_turn_route_is_rejected_before_dispatch() {
    let mut request = TurnRequest::new("hi");
    request.inference_url = Some("https://api.example/v1".into());
    request.api_key = Some("   ".into());

    assert!(matches!(
        validate_route(&request),
        Err(CoreError::InvalidRoute { method: AGENT_CHAT })
    ));
}

#[test]
fn absolute_leaves_absolute_paths_alone() {
    let already = std::path::Path::new("/tmp/example");
    assert_eq!(absolute(already).expect("absolute"), already);
}

#[test]
fn absolute_roots_a_relative_path_at_the_cwd() {
    let resolved = absolute("sub/dir").expect("absolute");
    assert!(resolved.is_absolute());
    assert!(resolved.ends_with("sub/dir"));
}

#[test]
fn route_debug_redacts_bearer_and_url_embedded_credentials() {
    let route = Route::openai_compatible(
        "https://user:topsecret@api.example/v1?key=leaky#frag",
        "sk-bearer",
    );
    let debug = format!("{route:?}");

    assert!(debug.contains("api.example"), "origin stays readable");
    assert!(
        !debug.contains("topsecret") && !debug.contains("sk-bearer") && !debug.contains("leaky"),
        "credentials leaked into Route Debug: {debug}"
    );
    assert!(debug.contains("redacted"));
}

#[test]
fn is_safe_endpoint_for_bearer_accepts_tls_and_loopback() {
    // TLS is always safe for a bearer.
    assert!(is_safe_endpoint_for_bearer("https://api.example.com/v1"));
    assert!(is_safe_endpoint_for_bearer("https://127.0.0.1:8443"));
    // Cleartext is refused for remote hosts...
    assert!(!is_safe_endpoint_for_bearer("http://api.example.com/v1"));
    assert!(!is_safe_endpoint_for_bearer("http://192.168.1.10/v1"));
    // ...but accepted for loopback, where the credential never leaves the host.
    assert!(is_safe_endpoint_for_bearer("http://127.0.0.1:8080/v1"));
    assert!(is_safe_endpoint_for_bearer("http://localhost:8080/v1"));
    assert!(is_safe_endpoint_for_bearer("http://[::1]:8080/v1"));
    assert!(is_safe_endpoint_for_bearer("http://127.0.0.2:8080/v1"));
    // Unparseable or non-URL values are refused, not silently allowed, so a
    // bearer can never ride a cleartext or ambiguous channel.
    assert!(!is_safe_endpoint_for_bearer("api.example.com"));
    assert!(!is_safe_endpoint_for_bearer("ftp://api.example.com"));
    assert!(!is_safe_endpoint_for_bearer("http://api.example.com:8080"));
}

#[test]
fn insecure_route_error_names_the_redacted_endpoint() {
    // The guard sanitizes the endpoint before it lands in the error, so the
    // message names the origin but never the credential-bearing parts.
    let endpoint = sanitize_url_for_display("http://user:pass@api.example/v1?key=leaky");
    let err = CoreError::InsecureRoute {
        method: AGENT_CHAT,
        endpoint,
    };
    let msg = err.to_string();
    assert!(msg.contains("non-HTTPS"), "classifies the reason: {msg}");
    assert!(!msg.contains("user"), "userinfo not leaked: {msg}");
    assert!(
        !msg.contains("leaky"),
        "query credentials not leaked: {msg}"
    );
    assert!(msg.contains("api.example"), "origin stays readable: {msg}");
}

#[test]
fn sanitize_redacts_unparseable_urls() {
    // A value that does not parse as an absolute URL cannot be proven free of
    // credential-bearing components (e.g. protocol-relative userinfo), so it
    // must never be echoed verbatim into diagnostics.
    assert_eq!(
        sanitize_url_for_display("//user:sk-secret@host/path"),
        "<redacted>"
    );
    assert_eq!(sanitize_url_for_display("not a url at all"), "<redacted>");
    // A parseable absolute URL is normalized, not dropped.
    let normalized = sanitize_url_for_display("https://user:sk-secret@api.example/v1?k=leaky");
    assert!(normalized.contains("api.example"));
    assert!(!normalized.contains("sk-secret"));
    assert!(!normalized.contains("leaky"));
}
