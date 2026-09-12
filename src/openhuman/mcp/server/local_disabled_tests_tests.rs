use super::*;

#[tokio::test]
async fn ensure_local_http_reports_unavailable_without_http_server() {
    let err = ensure_local_http()
        .await
        .expect_err("slim build without `http-server` must not start a server");
    assert!(
        err.to_string().contains("http-server feature"),
        "error must name the missing feature, got: {err}"
    );
}
