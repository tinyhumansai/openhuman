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

/// Like [`spawn_stub`], but also hands back the raw request bytes the client
/// sent so tests can assert on the method line, query string, headers and body.
///
/// Reads until the request is complete rather than taking whatever a single
/// `read` returns. One read is not one request: TCP may split the body from the
/// headers, or split the headers themselves. Callers assert on both halves
/// (`x-sdk-name` in the header block, JSON payloads in the body), so a short
/// read would surface as an assertion failure against a truncated prefix — a
/// flake that only reproduces under load.
pub(super) async fn spawn_stub_capture(
    response: Vec<u8>,
) -> (String, tokio::sync::oneshot::Receiver<String>) {
    /// Byte-substring search; `[u8]::contains` matches single elements only.
    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    /// `Content-Length` from a header block, or 0 when absent (e.g. GET).
    /// Header names are case-insensitive per RFC 9110.
    fn content_length(headers: &[u8]) -> usize {
        String::from_utf8_lossy(headers)
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse().ok())
            .unwrap_or(0)
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut raw: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];

        // Read to the end of the header block. A 0-length read (peer closed) or
        // an error ends the loop, so a misbehaving client cannot hang the test.
        let header_end = loop {
            match sock.read(&mut buf).await {
                Ok(0) | Err(_) => break raw.len(),
                Ok(n) => {
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(at) = find(&raw, b"\r\n\r\n") {
                        break at + 4;
                    }
                }
            }
        };

        // Then read until the declared body has arrived.
        let want = header_end + content_length(&raw[..header_end.min(raw.len())]);
        while raw.len() < want {
            match sock.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => raw.extend_from_slice(&buf[..n]),
            }
        }

        sock.write_all(&response).await.unwrap();
        sock.flush().await.unwrap();
        let _ = sock.shutdown().await;
        let _ = tx.send(String::from_utf8_lossy(&raw).to_string());
    });
    (format!("http://{addr}"), rx)
}

/// Build a minimal HTTP/1.1 JSON response with the given status line and body.
pub(super) fn http_json(status_line: &str, body: &str) -> Vec<u8> {
    format!(
        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}
