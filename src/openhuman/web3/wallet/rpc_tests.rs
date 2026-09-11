use super::redact_rpc_url;

#[test]
fn redact_rpc_url_strips_path_and_query() {
    assert_eq!(
        redact_rpc_url("https://user:pass@example.com/path/secret?apiKey=123"),
        "https://example.com"
    );
}

#[test]
fn redact_rpc_url_handles_invalid_values() {
    assert_eq!(redact_rpc_url("not a url"), "<invalid-url>");
}
