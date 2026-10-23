use super::redact_url_for_log;
use super::relay_bearer_header;

#[test]
fn bearer_header_present_for_real_token() {
    assert_eq!(
        relay_bearer_header(Some("tok123")).as_deref(),
        Some("Bearer tok123")
    );
    // Surrounding whitespace is trimmed before formatting.
    assert_eq!(
        relay_bearer_header(Some("  tok123  ")).as_deref(),
        Some("Bearer tok123")
    );
}

#[test]
fn bearer_header_absent_for_missing_or_blank_token() {
    assert_eq!(relay_bearer_header(None), None);
    assert_eq!(relay_bearer_header(Some("")), None);
    assert_eq!(relay_bearer_header(Some("   ")), None);
}

#[test]
fn redact_never_lets_credentials_query_or_path_into_logs() {
    // Userinfo, query, fragment and path must not survive into logs. The one URL
    // redactor is the security module's (origin only); with no module serving it
    // answers the suppression marker instead, which carries none of them either.
    let logged = redact_url_for_log("http://user:pass@192.168.1.74:7788/rpc/secret?token=t0k#frag");
    for leaked in ["user", "pass", "secret", "t0k", "frag"] {
        assert!(!logged.contains(leaked), "{leaked} leaked into {logged}");
    }
    // An unparseable URL is never echoed.
    assert!(!redact_url_for_log("not a url").contains("not a url"));
}

/// A non-loopback `http` URL carrying a bearer must be refused, and the
/// surfaced error must carry the redacted `scheme://host[:port]` form —
/// never the raw secret-bearing userinfo, path, or query (CWE-532).
#[cfg(feature = "gateways")]
#[tokio::test]
async fn insecure_transport_refusal_redacts_url() {
    let err = match super::post_json_rpc(
        "http://user:pass@192.168.1.74:7788/rpc/secret?token=t0k",
        Some("bearer-tok"),
        "body".to_string(),
    )
    .await
    {
        Err(e) => e,
        Ok(_) => panic!("insecure non-loopback + bearer must be refused"),
    };
    assert!(
        !err.contains("pass"),
        "raw userinfo leaked into refusal error: {err}"
    );
    assert!(
        !err.contains("t0k"),
        "raw query token leaked into refusal error: {err}"
    );
    assert!(
        !err.contains("/secret"),
        "raw path leaked into refusal error: {err}"
    );
    assert!(
        !err.contains("192.168.1.74"),
        "module-unavailable redaction must suppress the host too: {err}"
    );
}
