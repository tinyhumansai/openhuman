//! Unit tests for the CORS allowlist and header-emission logic in `jsonrpc.rs`.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use super::{is_origin_allowed, is_origin_allowed_with_extra, with_cors_headers};

fn ok_response() -> Response {
    (StatusCode::OK, "").into_response()
}

fn allow_origin(response: &Response) -> Option<String> {
    response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

#[test]
fn allows_tauri_webview_origins() {
    for origin in [
        "tauri://localhost",
        "http://tauri.localhost",
        "https://tauri.localhost",
    ] {
        assert!(is_origin_allowed(origin), "expected {origin} to be allowed");
        let r = with_cors_headers(ok_response(), Some(origin));
        assert_eq!(allow_origin(&r).as_deref(), Some(origin));
    }
}

#[test]
fn allows_loopback_with_any_port() {
    for origin in [
        "http://127.0.0.1:1420",
        "http://localhost:5173",
        "http://[::1]:4444",
        "http://localhost",
    ] {
        assert!(is_origin_allowed(origin), "expected {origin} to be allowed");
        let r = with_cors_headers(ok_response(), Some(origin));
        assert_eq!(allow_origin(&r).as_deref(), Some(origin));
    }
}

#[test]
fn rejects_disallowed_origins() {
    for origin in [
        "https://attacker.example",
        "http://evil.localhost.attacker.example",
        "https://127.0.0.1.attacker.example",
        // HTTPS variant of localhost is NOT a configuration we ship — refuse.
        "https://localhost",
        "null",
    ] {
        assert!(
            !is_origin_allowed(origin),
            "expected {origin} to be rejected"
        );
        let r = with_cors_headers(ok_response(), Some(origin));
        assert!(
            allow_origin(&r).is_none(),
            "disallowed origin {origin} leaked Access-Control-Allow-Origin"
        );
    }
}

#[test]
fn missing_origin_emits_no_acao_but_sets_vary() {
    let r = with_cors_headers(ok_response(), None);
    assert!(allow_origin(&r).is_none());
    assert_eq!(
        r.headers().get(header::VARY).and_then(|v| v.to_str().ok()),
        Some("Origin")
    );
}

#[test]
fn env_override_allows_extra_origins() {
    let extra_origins = Some("https://debug.internal, http://harness:9000");

    assert!(is_origin_allowed_with_extra(
        "https://debug.internal",
        extra_origins
    ));
    assert!(is_origin_allowed_with_extra(
        "http://harness:9000",
        extra_origins
    ));
    assert!(!is_origin_allowed_with_extra(
        "https://debug.internal.attacker.example",
        extra_origins
    ));
}

#[test]
fn preserves_existing_vary_values() {
    let mut response = ok_response();
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));

    let r = with_cors_headers(response, None);
    let values = r
        .headers()
        .get_all(header::VARY)
        .iter()
        .map(|v| v.to_str().unwrap_or("<invalid>"))
        .collect::<Vec<_>>();

    assert_eq!(values, vec!["Accept-Encoding", "Origin"]);
}

#[test]
fn env_override_does_not_allow_lookalike_suffixes() {
    assert!(!is_origin_allowed_with_extra(
        "https://debug.internal.attacker.example",
        Some("https://debug.internal")
    ));
}

#[test]
fn always_sets_methods_headers_and_max_age() {
    let r = with_cors_headers(ok_response(), Some("tauri://localhost"));
    let h = r.headers();
    assert_eq!(
        h.get(header::ACCESS_CONTROL_ALLOW_METHODS)
            .and_then(|v| v.to_str().ok()),
        Some("GET, POST, OPTIONS")
    );
    assert_eq!(
        h.get(header::ACCESS_CONTROL_ALLOW_HEADERS)
            .and_then(|v| v.to_str().ok()),
        Some("Content-Type, Authorization")
    );
    assert_eq!(
        h.get(header::ACCESS_CONTROL_MAX_AGE)
            .and_then(|v| v.to_str().ok()),
        Some("86400")
    );
}
