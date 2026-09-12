use super::*;

#[test]
fn is_invalid_token_error_matches_exact_sio_wire_shape() {
    assert!(is_invalid_token_error(
        "Socket.IO connect error: Invalid token"
    ));
}

#[test]
fn is_invalid_token_error_is_case_insensitive() {
    assert!(is_invalid_token_error(
        "socket.io connect error: invalid token"
    ));
    assert!(is_invalid_token_error(
        "SOCKET.IO CONNECT ERROR: INVALID TOKEN"
    ));
}

#[test]
fn is_invalid_token_error_requires_both_anchors() {
    // "invalid token" without the "socket.io connect error" prefix must
    // not fire — otherwise bare upstream 401s from other contexts would
    // trigger the fast-fail session-expiry path.
    assert!(!is_invalid_token_error("invalid token"));
    assert!(!is_invalid_token_error("auth error: invalid token"));
    // The SIO connect error prefix without the "invalid token" body must
    // not fire either — a server-side config error, for instance.
    assert!(!is_invalid_token_error(
        "socket.io connect error: namespace not found"
    ));
}

#[test]
fn is_invalid_token_error_returns_false_for_unrelated_errors() {
    assert!(!is_invalid_token_error(
        "WebSocket connect: connection refused"
    ));
    assert!(!is_invalid_token_error("EIO OPEN: timeout"));
    assert!(!is_invalid_token_error(""));
}

#[test]
fn static_provider_returns_token() {
    let provider = static_token_provider("my-token".to_string());
    assert_eq!(provider().unwrap(), "my-token");
}

#[test]
fn static_provider_rejects_empty_token() {
    let provider = static_token_provider("".to_string());
    assert!(provider().is_err());
    let provider2 = static_token_provider("   ".to_string());
    assert!(provider2().is_err());
}

#[test]
fn static_provider_returns_same_token_on_repeated_calls() {
    let provider = static_token_provider("tok-abc".to_string());
    // Simulates multiple reconnect attempts — must always return the same
    // cloned token (static provider semantics).
    assert_eq!(provider().unwrap(), provider().unwrap());
}
