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

#[test]
fn shared_security_url_corpus_pins_wallet_log_redactor() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/security-redaction-corpus.json"
    )))
    .unwrap();
    for case in corpus["urls"].as_array().unwrap() {
        assert_eq!(
            redact_rpc_url(case["input"].as_str().unwrap()),
            case["wallet"]
        );
    }
}
