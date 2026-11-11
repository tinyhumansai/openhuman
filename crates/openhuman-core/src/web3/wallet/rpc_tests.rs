use super::{redact_rpc_url, rest_get_text, rest_post_text, rpc_call_to};

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

/// A port nothing listens on, so every request fails in transport.
fn refused_endpoint(path: &str) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    format!("http://127.0.0.1:{port}{path}")
}

const KEYED_PATH: &str = "/v2/secret-path-key?api-key=secret-query-key";

fn assert_no_endpoint_secret(error: &str) {
    assert!(
        error.contains("transport failed"),
        "unexpected error: {error}"
    );
    assert!(
        !error.contains("secret-query-key") && !error.contains("secret-path-key"),
        "a transport error must not carry the endpoint URL: {error}"
    );
}

#[tokio::test]
async fn a_json_rpc_transport_error_does_not_carry_the_endpoint_url() {
    let error = rpc_call_to::<serde_json::Value>(
        &refused_endpoint(KEYED_PATH),
        "getTokenAccountsByOwner",
        serde_json::json!([]),
    )
    .await
    .unwrap_err();
    assert_no_endpoint_secret(&error);
}

#[tokio::test]
async fn a_rest_transport_error_does_not_carry_the_endpoint_url() {
    assert_no_endpoint_secret(
        &rest_get_text(&refused_endpoint(KEYED_PATH))
            .await
            .unwrap_err(),
    );
    assert_no_endpoint_secret(
        &rest_post_text(&refused_endpoint(KEYED_PATH), "00", "text/plain")
            .await
            .unwrap_err(),
    );
}
