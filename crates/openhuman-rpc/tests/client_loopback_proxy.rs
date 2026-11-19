//! The local core must not be reached through the process proxy.
//!
//! A proxy is not the local core: it forwards the request to a hop that
//! cannot reach the caller's own `127.0.0.1`, and that hop answers `502`
//! with an empty body. This is what the desktop app reports as "core rpc
//! returned a non-JSON body (http 502)".
//!
//! These cases live in their own binary on purpose: the proxy variables they
//! set are process-global, and they must not leak into the crate's unit
//! tests.

#![cfg(feature = "http-client")]

use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard};

use openhuman_rpc::post_json_rpc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

/// Serializes the cases in this binary around the proxy variables.
static PROXY_ENV_LOCK: Mutex<()> = Mutex::new(());

/// Sets the proxy variables for the lifetime of the guard and restores them
/// afterwards. `NO_PROXY` is cleared: an inherited exemption would hide the
/// very case under test.
struct ProxyEnv {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl ProxyEnv {
    fn set(proxy_url: &str) -> Self {
        let lock = PROXY_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut saved = Vec::new();
        for key in ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
            saved.push((key, std::env::var_os(key)));
            std::env::set_var(key, proxy_url);
        }
        for key in ["NO_PROXY", "no_proxy"] {
            saved.push((key, std::env::var_os(key)));
            std::env::remove_var(key);
        }
        Self { _lock: lock, saved }
    }
}

impl Drop for ProxyEnv {
    fn drop(&mut self) {
        for (key, value) in self.saved.iter().rev() {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

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
    let announced = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.trim().eq_ignore_ascii_case("content-length") {
                value.trim().parse().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);
    let mut body = raw.len() - head_end;
    while body < announced {
        let read = socket.read(&mut chunk).await.expect("read request body");
        assert!(read > 0, "the client closed before sending the body");
        body += read;
    }
    head
}

/// A stand-in for the local core: answers one JSON-RPC request.
async fn serve_local_core() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind core listener");
    let address = listener.local_addr().expect("core address");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept core request");
        let _ = read_request(&mut socket).await;
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = socket.write_all(response.as_bytes()).await;
    });
    format!("http://{address}/rpc")
}

/// A stand-in for a proxy that cannot reach the caller's loopback interface:
/// it answers `502` with an empty body, the way Clash does.
async fn serve_proxy() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy listener");
    let address = listener.local_addr().expect("proxy address");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept proxy request");
        let _ = read_request(&mut socket).await;
        let response = "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let _ = socket.write_all(response.as_bytes()).await;
    });
    format!("http://{address}")
}

/// A stand-in for a proxy that works, recording the request it was handed.
async fn serve_recording_proxy() -> (String, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy listener");
    let address = listener.local_addr().expect("proxy address");
    let (sender, receiver) = oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept proxy request");
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
    (format!("http://{address}"), receiver)
}

#[tokio::test]
async fn the_local_core_is_reached_without_the_proxy() {
    let core_url = serve_local_core().await;
    let proxy_url = serve_proxy().await;
    let _env = ProxyEnv::set(&proxy_url);

    let response = post_json_rpc(&core_url, Some("token"), "{}")
        .await
        .expect("the local core must be reachable");
    assert_eq!(response.status, 200, "body: {}", response.body);
    assert!(
        response.body.contains("\"ok\":true"),
        "body: {}",
        response.body
    );
}

#[tokio::test]
async fn a_remote_target_still_goes_through_the_proxy() {
    let (proxy_url, request) = serve_recording_proxy().await;
    let _env = ProxyEnv::set(&proxy_url);

    // A TEST-NET address, so nothing resolves or answers for it: the proxy
    // is the only thing that can.
    let response = post_json_rpc("http://198.51.100.7/rpc", None, "{}")
        .await
        .expect("the proxy answers for the remote target");
    assert_eq!(response.status, 200, "body: {}", response.body);
    let head = request.await.expect("the proxy received the request");
    assert!(head.contains("198.51.100.7"), "{head}");
}
