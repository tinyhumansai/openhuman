use super::*;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

/// Read a whole HTTP request: the head, plus the body the head announces.
///
/// The body has to be consumed as well: a socket closed with unread bytes
/// in its receive buffer resets the connection, and the client would see
/// the reset instead of the response.
async fn read_request(socket: &mut TcpStream) -> String {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        let read = socket.read(&mut chunk).await.expect("read request");
        assert!(read > 0, "the client closed before sending a request");
        raw.extend_from_slice(&chunk[..read]);
        if let Some(end) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
    let announced = content_length(&head);
    let mut body = raw.len() - head_end;
    while body < announced {
        let read = socket.read(&mut chunk).await.expect("read request body");
        assert!(read > 0, "the client closed before sending the body");
        body += read;
    }
    head
}

/// The `Content-Length` a request head announces, or 0.
fn content_length(head: &str) -> usize {
    head.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.trim().eq_ignore_ascii_case("content-length") {
                value.trim().parse().ok()
            } else {
                None
            }
        })
        .unwrap_or(0)
}

/// Serve one minimal JSON-RPC response on a loopback port, and hand back the
/// request head the client sent.
///
/// A local socket, not a backend: the point of the test is that the client
/// reaches a loopback endpoint at all.
async fn serve_one_rpc_response() -> (String, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback listener");
    let address = listener.local_addr().expect("listener address");
    let (sender, receiver) = oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept connection");
        let head = read_request(&mut socket).await;
        let _ = sender.send(head);
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = socket.write_all(response.as_bytes()).await;
    });
    (format!("http://{address}/rpc"), receiver)
}

#[test]
fn loopback_urls_are_recognized() {
    assert!(is_loopback_url("http://127.0.0.1:7788/rpc"));
    assert!(is_loopback_url("http://localhost:7788/rpc"));
    assert!(is_loopback_url("http://LOCALHOST:7788/rpc"));
    assert!(is_loopback_url("http://[::1]:7788/rpc"));
    assert!(is_loopback_url("http://core.localhost:7788/rpc"));
    assert!(is_loopback_url("https://127.0.0.1:7788/rpc"));
    // The whole 127.0.0.0/8 range is loopback, not only 127.0.0.1.
    assert!(is_loopback_url("http://127.0.0.2:7788/rpc"));
    assert!(is_loopback_url("http://127.1.2.3:7788/rpc"));
}

#[test]
fn remote_and_unparseable_urls_are_not_loopback() {
    assert!(!is_loopback_url("http://core.example.com/rpc"));
    assert!(!is_loopback_url("http://192.168.1.74:7788/rpc"));
    assert!(!is_loopback_url("http://127.0.0.1.example.com:7788/rpc"));
    assert!(!is_loopback_url("http://0.0.0.0:7788/rpc"));
    assert!(!is_loopback_url("not a url"));
    assert!(!is_loopback_url(""));
}

/// The local core is reachable on loopback even when the environment names
/// a proxy: a proxy forwards the request to a hop that cannot reach the
/// caller's own `127.0.0.1`.
#[tokio::test]
async fn post_json_rpc_reaches_a_loopback_endpoint() {
    let (url, _request) = serve_one_rpc_response().await;
    let response = post_json_rpc(&url, Some("token"), "{}")
        .await
        .expect("loopback rpc should succeed");
    assert_eq!(response.status, 200);
    assert!(response.body.contains("\"ok\":true"), "{}", response.body);
}

#[tokio::test]
async fn post_json_rpc_reports_a_refused_endpoint() {
    // A port that was just released: the kernel hands it out, so nothing is
    // listening on it. A fixed port would depend on the machine instead.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let port = listener.local_addr().expect("the bound address").port();
    drop(listener);
    let error = post_json_rpc(&format!("http://127.0.0.1:{port}/rpc"), None, "{}")
        .await
        .expect_err("a refused endpoint must surface as an error");
    assert!(error.contains("failed"), "{error}");
}

#[tokio::test]
async fn bearer_header_is_sent_when_a_token_is_given() {
    let (url, request) = serve_one_rpc_response().await;
    let response = post_json_rpc(&url, Some("token"), "{}")
        .await
        .expect("loopback rpc should succeed");
    assert_eq!(response.status, 200);
    let head = request.await.expect("the client sent a request");
    assert!(head.contains("authorization:"), "{head}");
    assert!(head.contains("Bearer token"), "{head}");
}
