use super::*;
use serde_json::json;

#[test]
fn parses_the_wrapped_success_envelope() {
    let raw = json!({ "success": true, "data": { "signedUrl": "wss://x", "agentId": "a1", "userToken": "tok" } });
    let out = parse_signed_url_response(&raw).unwrap();
    assert_eq!(out.signed_url, "wss://x");
    assert_eq!(out.agent_id, "a1");
    assert_eq!(out.user_token, "tok");
}

#[test]
fn tolerates_a_bare_object() {
    let raw = json!({ "signedUrl": "wss://y", "agentId": "a2" });
    let out = parse_signed_url_response(&raw).unwrap();
    assert_eq!(out.signed_url, "wss://y");
    assert_eq!(out.agent_id, "a2");
}

#[test]
fn user_token_defaults_empty_against_an_older_backend() {
    let raw = json!({ "data": { "signedUrl": "wss://y", "agentId": "a2" } });
    let out = parse_signed_url_response(&raw).unwrap();
    assert_eq!(out.user_token, "");
}

#[test]
fn secure_url_guard_allows_https_and_loopback_http_only() {
    assert!(ensure_secure_backend_url("https://api.tinyhumans.ai").is_ok());
    assert!(ensure_secure_backend_url("http://localhost:5005").is_ok());
    assert!(ensure_secure_backend_url("http://127.0.0.1:5005/").is_ok());
    let err = ensure_secure_backend_url("http://api.tinyhumans.ai").unwrap_err();
    assert!(err.contains("non-HTTPS"), "{err}");
}

#[test]
fn secure_url_guard_allows_ipv6_loopback() {
    // Bracketed IPv6 loopback must be accepted — the previous first-`:`
    // split turned `[::1]:5005` into `"["` and wrongly rejected it.
    assert!(ensure_secure_backend_url("http://[::1]:5005").is_ok());
    assert!(ensure_secure_backend_url("http://[::1]:5005/").is_ok());
    assert!(ensure_secure_backend_url("http://[::1]").is_ok());
    // A non-loopback bracketed IPv6 host is still rejected over http://.
    let err = ensure_secure_backend_url("http://[2001:db8::1]:5005").unwrap_err();
    assert!(err.contains("non-HTTPS"), "{err}");
}

#[test]
fn errors_when_signed_url_is_absent() {
    let raw = json!({ "data": { "agentId": "a3" } });
    let err = parse_signed_url_response(&raw).unwrap_err();
    assert!(err.contains("no signed_url"), "{err}");
}

#[test]
fn result_serializes_snake_case_for_the_wire() {
    let json = serde_json::to_value(VoiceAgentSignedUrl {
        signed_url: "wss://z".into(),
        agent_id: "a4".into(),
        user_token: "tok".into(),
    })
    .unwrap();
    assert_eq!(json.get("signed_url").unwrap(), "wss://z");
    assert_eq!(json.get("agent_id").unwrap(), "a4");
    assert_eq!(json.get("user_token").unwrap(), "tok");
}
