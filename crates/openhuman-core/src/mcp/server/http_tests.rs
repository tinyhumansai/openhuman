use super::*;
use axum::body::to_bytes;
use std::collections::BTreeMap;

#[tokio::test]
async fn module_http_metadata_controls_status_headers_and_response_body() {
    let response = response_from_metadata(
        tinymcp_bus::ServerHttpResponseMetadata {
            status_code: 202,
            headers: BTreeMap::from([
                ("mcp-session-id".to_owned(), "session-7".to_owned()),
                ("mcp-protocol-version".to_owned(), "2025-11-25".to_owned()),
            ]),
            content_type: Some("application/json".to_owned()),
            error_body: None,
        },
        r#"{"jsonrpc":"2.0","id":4,"result":{}}"#.to_owned(),
    );

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(response.headers()[HEADER_SESSION_ID], "session-7");
    assert_eq!(response.headers()["mcp-protocol-version"], "2025-11-25");
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        r#"{"jsonrpc":"2.0","id":4,"result":{}}"#
    );
}

#[tokio::test]
async fn module_http_error_metadata_preserves_plain_text_status_and_body() {
    let response = response_from_metadata(
        tinymcp_bus::ServerHttpResponseMetadata {
            status_code: 400,
            headers: BTreeMap::new(),
            content_type: Some("text/plain; charset=utf-8".to_owned()),
            error_body: Some("missing or invalid Mcp-Session-Id header".to_owned()),
        },
        String::new(),
    );

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        "missing or invalid Mcp-Session-Id header"
    );
}

#[tokio::test]
async fn invalid_module_http_status_fails_closed_as_internal_server_error() {
    let response = response_from_metadata(
        tinymcp_bus::ServerHttpResponseMetadata {
            status_code: 99,
            headers: BTreeMap::new(),
            content_type: None,
            error_body: Some("bad status".to_owned()),
        },
        String::new(),
    );

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        "bad status"
    );
}
