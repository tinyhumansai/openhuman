use super::redact_url_for_log;
use serde_json::Value;

#[test]
fn shared_characterization_corpus_tracks_rpc_url_redaction() {
    let corpus: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/security-redaction-extended-corpus.json"
    )))
    .unwrap();
    for case in corpus["urls"].as_array().unwrap() {
        let input = case["input_parts"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .map(|part| part.as_str().unwrap())
                    .collect::<String>()
            })
            .unwrap_or_else(|| case["input"].as_str().unwrap().to_string());
        let needle = case["needle"].as_str().unwrap();
        let expected = case["removed_by"]
            .as_array()
            .unwrap()
            .iter()
            .any(|sink| sink.as_str() == Some("rpc_url"));
        assert_eq!(
            !redact_url_for_log(&input).contains(needle),
            expected,
            "case={}",
            case["case"]
        );
    }
}

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

/// Serve one redirect to a given target, and hand back the served URL.
async fn serve_one_redirect_to(target: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback listener");
    let address = listener.local_addr().expect("listener address");
    let target = target.to_string();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept connection");
        let _ = read_request(&mut socket).await;
        let response = format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let _ = socket.write_all(response.as_bytes()).await;
    });
    format!("http://{address}/rpc")
}

/// Serve redirects to the served URL itself, forever.
async fn serve_a_redirect_loop() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback listener");
    let address = listener.local_addr().expect("listener address");
    let target = format!("http://{address}/rpc");
    let location = target.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.expect("accept connection");
            let _ = read_request(&mut socket).await;
            let response = format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    target
}

/// Serve one redirect to a remote address, and hand back its URL.
async fn serve_one_redirect() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback listener");
    let address = listener.local_addr().expect("listener address");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept connection");
        let _ = read_request(&mut socket).await;
        let response = "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://198.51.100.7/rpc\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let _ = socket.write_all(response.as_bytes()).await;
    });
    format!("http://{address}/rpc")
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
    // An IPv4-mapped IPv6 address reaches the same loopback interface.
    assert!(is_loopback_url("http://[::ffff:127.0.0.1]:7788/rpc"));
    assert!(is_loopback_url("http://[::ffff:7f00:1]:7788/rpc"));
}

#[test]
fn remote_and_unparseable_urls_are_not_loopback() {
    assert!(!is_loopback_url("http://core.example.com/rpc"));
    assert!(!is_loopback_url("http://192.168.1.74:7788/rpc"));
    assert!(!is_loopback_url("http://127.0.0.1.example.com:7788/rpc"));
    assert!(!is_loopback_url("http://0.0.0.0:7788/rpc"));
    // A mapped address for a routable IPv4 address is not loopback.
    assert!(!is_loopback_url("http://[::ffff:192.168.1.74]:7788/rpc"));
    assert!(!is_loopback_url("not a url"));
    assert!(!is_loopback_url(""));
    // A URL without a host has no loopback interface either.
    assert!(!is_loopback_url("file:///tmp/rpc"));
}

/// A logger that keeps the messages it is given, so a test can read them.
struct CapturingLogger;

static LOGGED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

impl log::Log for CapturingLogger {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        if let Ok(mut lines) = LOGGED.lock() {
            lines.push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

/// Install the capturing logger, once per test process.
fn capture_logs() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        log::set_logger(&CapturingLogger).expect("the logger is installed once");
        log::set_max_level(log::LevelFilter::Debug);
    });
}

/// The messages logged since the last call.
fn logged_lines() -> Vec<String> {
    LOGGED.lock().map(|lines| lines.clone()).unwrap_or_default()
}

/// Check loopback connectivity. Proxy bypass is covered by
/// `tests/client_loopback_proxy.rs`.
#[tokio::test]
async fn the_proxy_bypass_is_logged_with_the_redacted_url() {
    capture_logs();
    let (url, _request) = serve_one_rpc_response().await;
    post_json_rpc(&url, None, "{}")
        .await
        .expect("loopback rpc should succeed");
    let lines = logged_lines();
    let bypass = lines
        .iter()
        .find(|line| line.contains("bypassing the proxy"))
        .expect("the bypass decision is logged");
    assert!(bypass.contains("127.0.0.1"), "{bypass}");
    // The path is not part of the log line.
    assert!(!bypass.contains("/rpc"), "{bypass}");
}

#[tokio::test]
async fn a_remote_target_is_logged_as_using_the_ambient_proxy_settings() {
    capture_logs();
    // A non-loopback address with nothing listening on it: the request fails
    // right away, and the decision is logged before the request is sent.
    let _ = post_json_rpc("http://192.168.110.51:45999/rpc", None, "{}").await;
    let lines = logged_lines();
    let ambient = lines
        .iter()
        .find(|line| line.contains("ambient proxy settings"))
        .expect("the ambient proxy decision is logged");
    assert!(ambient.contains("192.168.110.51"), "{ambient}");
    assert!(!ambient.contains("/rpc"), "{ambient}");
}

#[tokio::test]
async fn a_redirect_off_the_loopback_interface_is_not_followed() {
    // A loopback endpoint that answers with a redirect to a remote host: the
    // client has no proxy, so following it would reach that host directly.
    let url = serve_one_redirect().await;
    let response = post_json_rpc(&url, None, "{}")
        .await
        .expect("the redirect response reaches the caller");
    assert_eq!(response.status, 307, "body: {}", response.body);
}

#[tokio::test]
async fn a_redirect_that_stays_on_the_loopback_interface_is_followed() {
    let target = serve_one_rpc_response().await.0;
    let url = serve_one_redirect_to(&target).await;
    let response = post_json_rpc(&url, None, "{}")
        .await
        .expect("the loopback redirect reaches the target");
    assert_eq!(response.status, 200, "body: {}", response.body);
}

#[tokio::test]
async fn a_redirect_loop_on_the_loopback_interface_gives_up() {
    let url = serve_a_redirect_loop().await;
    let error = post_json_rpc(&url, None, "{}")
        .await
        .expect_err("a redirect loop does not resolve");
    // The client gives up instead of following the loop forever.
    assert!(error.contains("error following redirect"), "{error}");
}

#[tokio::test]
async fn post_json_rpc_reaches_a_loopback_endpoint() {
    let (url, _request) = serve_one_rpc_response().await;
    let response = post_json_rpc(&url, Some("token"), "{}")
        .await
        .expect("loopback rpc should succeed");
    assert_eq!(response.status, 200);
    assert!(response.body.contains("\"ok\":true"), "{}", response.body);
}

/// A loopback port that nothing is listening on.
///
/// The port comes from the kernel, so the case does not depend on what the
/// machine happens to run, but releasing the port opens a window in which
/// another process could take it. The port is therefore checked before it is
/// used, and another one is drawn when the check finds a listener.
fn closed_loopback_port() -> u16 {
    for _ in 0..16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let port = listener.local_addr().expect("the bound address").port();
        drop(listener);
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            // Nothing took the port: the next connection is refused.
            Err(_) => return port,
            // Something is listening on it after all; draw another one.
            Ok(socket) => drop(socket),
        }
    }
    panic!("no closed loopback port could be found");
}

#[tokio::test]
async fn post_json_rpc_reports_a_refused_endpoint() {
    for _ in 0..16 {
        let port = closed_loopback_port();
        match post_json_rpc(&format!("http://127.0.0.1:{port}/rpc"), None, "{}").await {
            Ok(response) => {
                // The port was taken by another process between the check and
                // the request; draw another one.
                let _ = response;
                continue;
            }
            Err(error) => {
                assert!(error.contains("failed"), "{error}");
                return;
            }
        }
    }
    panic!("no refused loopback endpoint could be reached");
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
