//! Unit and integration tests for the Medulla client, split by surface:
//! [`decode_tests`] covers envelope/error/run-result JSON decoding;
//! [`sse_tests`] covers the SSE parser, dedupe cursor, and streaming;
//! [`integration_tests`] covers the HTTP endpoint surface against a TCP stub.
//!
//! Shared TCP-stub helpers used by more than one child module live here.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

mod decode_tests;
mod integration_tests;
mod sse_tests;

/// Bind a stub server that handles one connection: drain the request, then
/// write `response` verbatim and close. Returns the bound address.
pub(super) async fn spawn_stub(response: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // Read whatever the client sent (headers, possibly body); ignore it.
        let mut buf = [0u8; 4096];
        let _ = sock.read(&mut buf).await;
        sock.write_all(&response).await.unwrap();
        sock.flush().await.unwrap();
        let _ = sock.shutdown().await;
    });
    format!("http://{addr}")
}

/// Build a minimal HTTP/1.1 JSON response with the given status line and body.
pub(super) fn http_json(status_line: &str, body: &str) -> Vec<u8> {
    format!(
        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}
