//! Loopback HTTP listener for OAuth / magic-link callbacks (RFC 8252).
//!
//! Used as the preferred desktop redirect target ahead of the `openhuman://`
//! deep link: the frontend asks the shell to bind a one-shot HTTP server on a
//! fixed loopback port, hands the resulting URL to the backend as
//! `redirectUri`, and waits for the `loopback-oauth-callback` Tauri event.
//!
//! Lifecycle is spawn-on-demand: each call to
//! [`start_loopback_oauth_listener`] supersedes any previously-running
//! listener, binds `127.0.0.1:<port>`, accepts connections until either the
//! state-matching `/auth` request arrives or `timeout_secs` elapses, then
//! shuts the listener down. If bind fails (port already in use), the command
//! returns an error and the caller falls back to the deep-link path.
//!
//! Only the `/auth` path is honored — favicons and stray requests get a
//! 404 and keep the loop alive. The state nonce is generated in the shell
//! and returned to the caller; the backend must echo it back as `state=` on
//! the redirect so a hostile page on the same loopback origin cannot fake a
//! callback.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use rand::RngCore;
use serde::Serialize;
use tauri::Emitter;

use crate::AppRuntime;
type AppHandle = tauri::AppHandle<AppRuntime>;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket};
use tokio::sync::oneshot;
use tokio::time::timeout;

const LOOPBACK_CALLBACK_EVENT: &str = "loopback-oauth-callback";
const READ_BUFFER_BYTES: usize = 8 * 1024;
const PER_CONNECTION_READ_TIMEOUT: Duration = Duration::from_secs(5);

struct ActiveListener {
    id: u64,
    tx: oneshot::Sender<()>,
    done: Option<tauri::async_runtime::JoinHandle<()>>,
}

static NEXT_LISTENER_ID: AtomicU64 = AtomicU64::new(1);
static ACTIVE_LISTENER: Mutex<Option<ActiveListener>> = Mutex::new(None);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StartResult {
    /// Full redirect URI the backend should redirect to, e.g.
    /// `http://127.0.0.1:53824/auth`. State is appended by the caller.
    /// Serializes as `redirectUri` so the TS-side `result.redirectUri`
    /// destructure works.
    pub redirect_uri: String,
    /// State nonce the backend must echo back as `?state=<value>`.
    pub state: String,
}

#[derive(Serialize, Clone)]
struct CallbackPayload {
    /// Full callback URL including query string. Frontend re-uses the existing
    /// `handleAuthDeepLink` parser by converting it to an `openhuman://` URL.
    url: String,
}

/// Signal the active listener to stop and return its join handle so the caller
/// can await its full teardown — critical when re-binding a fixed port, since
/// macOS releases the socket only after the owning task drops the listener.
fn take_active_listener() -> Option<tauri::async_runtime::JoinHandle<()>> {
    if let Ok(mut guard) = ACTIVE_LISTENER.lock() {
        if let Some(mut active) = guard.take() {
            let _ = active.tx.send(());
            return active.done.take();
        }
    }
    None
}

fn cancel_active_listener() {
    let _ = take_active_listener();
}

fn install_active_listener(
    id: u64,
    tx: oneshot::Sender<()>,
    done: tauri::async_runtime::JoinHandle<()>,
) {
    if let Ok(mut guard) = ACTIVE_LISTENER.lock() {
        if let Some(mut old) = guard.replace(ActiveListener {
            id,
            tx,
            done: Some(done),
        }) {
            let _ = old.tx.send(());
            // The previous listener's join handle is dropped here without an
            // await — only the new-start path needs to await teardown. Stray
            // installs (none today) would simply leak the wait, not break.
            old.done.take();
        }
    }
}

/// Only clear the global slot if it still belongs to this listener's id.
/// A superseded listener's exit must NOT wipe out the newer sender installed
/// by the start that cancelled it.
fn clear_active_listener(id: u64) {
    if let Ok(mut guard) = ACTIVE_LISTENER.lock() {
        if guard.as_ref().map(|active| active.id) == Some(id) {
            *guard = None;
        }
    }
}

/// Bind a loopback TCP listener on the given port (or 0 for ephemeral). Sets
/// SO_REUSEADDR so re-binding the same port soon after a previous listener
/// dropped doesn't trip EADDRINUSE on the TIME_WAIT window.
fn bind_loopback(port: u16) -> Result<TcpListener, String> {
    let sock_addr: std::net::SocketAddr = format!("127.0.0.1:{port}")
        .parse()
        .map_err(|err| format!("parse 127.0.0.1:{port} failed: {err}"))?;
    let socket = TcpSocket::new_v4().map_err(|err| format!("TcpSocket::new_v4 failed: {err}"))?;
    socket
        .set_reuseaddr(true)
        .map_err(|err| format!("set_reuseaddr failed: {err}"))?;
    socket
        .bind(sock_addr)
        .map_err(|err| format!("bind 127.0.0.1:{port} failed: {err}"))?;
    socket
        .listen(16)
        .map_err(|err| format!("listen on 127.0.0.1:{port} failed: {err}"))
}

fn random_state_nonce() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Parse the request target (path + query) out of an HTTP/1.x request head.
fn parse_request_target(head: &str) -> Option<&str> {
    let first_line = head.split("\r\n").next()?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    if method.eq_ignore_ascii_case("GET") {
        Some(target)
    } else {
        None
    }
}

/// Return the value of `state=` in a query string, if present.
fn extract_state(query: &str) -> Option<&str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == "state")
        .map(|(_, v)| v)
}

/// Outcome of classifying one HTTP request against the loopback accept loop.
/// Extracted so routing logic can be unit-tested without a live `AppHandle`.
#[derive(Debug, PartialEq)]
enum RequestOutcome {
    /// `/auth` matched and state is valid. Caller should send 200, emit callback.
    AuthCallback { callback_url: String },
    /// `/auth` matched but `state=` was missing or wrong. Caller sends 400.
    StateMismatch,
    /// Path is not `/auth`. Caller sends 404.
    NotFound,
    /// Method is not GET. Caller sends 405.
    MethodNotAllowed,
}

/// Classify one HTTP/1.x request received by the loopback accept loop.
fn classify_request(head: &str, expected_state: &str, bound_port: u16) -> RequestOutcome {
    let target = match parse_request_target(head) {
        Some(t) => t.to_string(),
        None => return RequestOutcome::MethodNotAllowed,
    };

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target.as_str(), ""),
    };

    if path != "/auth" {
        return RequestOutcome::NotFound;
    }

    match extract_state(query) {
        Some(s) if s == expected_state => {
            let callback_url = format!("http://127.0.0.1:{bound_port}{target}");
            RequestOutcome::AuthCallback { callback_url }
        }
        _ => RequestOutcome::StateMismatch,
    }
}

const SUCCESS_BODY: &str = "<!doctype html><meta charset=utf-8><title>Signed in</title>\
<body style=\"font-family:system-ui;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;color:#1c1c1e;background:#f5f5f7\">\
<div style=\"text-align:center\"><h2 style=\"margin:0 0 8px\">You're signed in.</h2>\
<p style=\"margin:0;color:#6e6e73\">You can close this tab and return to OpenHuman.</p></div>\
<script>setTimeout(function(){window.close()},250)</script></body>";

fn http_response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {len}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        len = body.len(),
    )
    .into_bytes()
}

#[tauri::command]
pub async fn start_loopback_oauth_listener(
    app: AppHandle,
    port: u16,
    timeout_secs: u64,
) -> Result<StartResult, String> {
    // Await the previous listener's task ending so the OS has actually
    // released the fixed loopback port. SO_REUSEADDR alone is not enough on
    // macOS — the prior socket must be dropped first.
    if let Some(done) = take_active_listener() {
        let _ = done.await;
    }

    // Prefer the caller's requested port (so the backend allowlist, if any,
    // matches) but fall back to an ephemeral OS-assigned port if the requested
    // one is taken by another process (stale openhuman, second instance,
    // unrelated service). The backend `redirectUri` whitelist restricts host
    // but not port, so an ephemeral fallback is safe.
    let listener: TcpListener = match bind_loopback(port) {
        Ok(l) => l,
        Err(primary_err) => {
            log::warn!(
                "[loopback-oauth] bind on requested port {port} failed ({primary_err}); retrying on ephemeral port"
            );
            bind_loopback(0).map_err(|err| {
                format!(
                    "bind 127.0.0.1:{port} failed ({primary_err}); ephemeral fallback also failed: {err}"
                )
            })?
        }
    };
    // Use the listener's actual bound port for the emitted callback URL so
    // the frontend rewrite (`^https?://127.0.0.1:\d+/auth`) always matches,
    // even if a future change moves to port 0.
    let bound_port = listener
        .local_addr()
        .map(|addr| addr.port())
        .unwrap_or(port);
    log::info!("[loopback-oauth] listening on 127.0.0.1:{bound_port}");

    let state = random_state_nonce();
    let redirect_uri = format!("http://127.0.0.1:{bound_port}/auth");

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    let listener_id = NEXT_LISTENER_ID.fetch_add(1, Ordering::Relaxed);

    let expected_state = state.clone();
    let done = tauri::async_runtime::spawn(async move {
        let lifetime = Duration::from_secs(timeout_secs.max(1));
        let run = run_accept_loop(listener, app, expected_state, bound_port, cancel_rx);
        match timeout(lifetime, run).await {
            Ok(()) => log::info!("[loopback-oauth] listener finished"),
            Err(_) => log::warn!(
                "[loopback-oauth] listener timed out after {}s",
                lifetime.as_secs()
            ),
        }
        clear_active_listener(listener_id);
    });
    install_active_listener(listener_id, cancel_tx, done);

    Ok(StartResult {
        redirect_uri,
        state,
    })
}

#[tauri::command]
pub async fn stop_loopback_oauth_listener() -> Result<(), String> {
    cancel_active_listener();
    Ok(())
}

async fn run_accept_loop(
    listener: TcpListener,
    app: AppHandle,
    expected_state: String,
    bound_port: u16,
    mut cancel_rx: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut cancel_rx => {
                log::debug!("[loopback-oauth] cancelled by new start or explicit stop");
                return;
            }
            accept = listener.accept() => {
                let (mut socket, peer) = match accept {
                    Ok(pair) => pair,
                    Err(err) => {
                        log::warn!("[loopback-oauth] accept failed: {err}");
                        continue;
                    }
                };
                if !peer.ip().is_loopback() {
                    log::warn!("[loopback-oauth] rejecting non-loopback peer {peer}");
                    let _ = socket.shutdown().await;
                    continue;
                }

                let mut buf = vec![0u8; READ_BUFFER_BYTES];
                let read = match timeout(PER_CONNECTION_READ_TIMEOUT, socket.read(&mut buf)).await {
                    Ok(Ok(n)) => n,
                    Ok(Err(err)) => {
                        log::debug!("[loopback-oauth] read error from {peer}: {err}");
                        continue;
                    }
                    Err(_) => {
                        log::debug!("[loopback-oauth] read timeout from {peer}");
                        continue;
                    }
                };
                if read == 0 {
                    continue;
                }

                let head = String::from_utf8_lossy(&buf[..read]);
                match classify_request(&head, &expected_state, bound_port) {
                    RequestOutcome::MethodNotAllowed => {
                        let _ = socket
                            .write_all(&http_response("405 Method Not Allowed", "method not allowed"))
                            .await;
                    }
                    RequestOutcome::NotFound => {
                        let _ = socket
                            .write_all(&http_response("404 Not Found", "not found"))
                            .await;
                    }
                    RequestOutcome::StateMismatch => {
                        log::warn!(
                            "[loopback-oauth] /auth with missing or mismatched state — ignoring"
                        );
                        let _ = socket
                            .write_all(&http_response("400 Bad Request", "state mismatch"))
                            .await;
                    }
                    RequestOutcome::AuthCallback { callback_url } => {
                        let _ = socket.write_all(&http_response("200 OK", SUCCESS_BODY)).await;
                        let _ = socket.flush().await;
                        if let Err(err) =
                            app.emit(LOOPBACK_CALLBACK_EVENT, CallbackPayload { url: callback_url })
                        {
                            log::warn!("[loopback-oauth] emit callback event failed: {err}");
                        }
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_get_request_target() {
        let head = "GET /auth?token=abc&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(
            parse_request_target(head),
            Some("/auth?token=abc&state=xyz")
        );
    }

    #[test]
    fn rejects_non_get_methods() {
        let head = "POST /auth HTTP/1.1\r\n\r\n";
        assert_eq!(parse_request_target(head), None);
    }

    #[test]
    fn extracts_state_value() {
        assert_eq!(extract_state("token=abc&state=xyz"), Some("xyz"));
        assert_eq!(extract_state("state=only"), Some("only"));
        assert_eq!(extract_state("token=abc"), None);
        assert_eq!(extract_state(""), None);
    }

    #[tokio::test]
    async fn random_state_is_32_hex_chars() {
        let s = random_state_nonce();
        assert_eq!(s.len(), 32);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ── classify_request ────────────────────────────────────────────────────

    fn auth_head(query: &str) -> String {
        format!("GET /auth{query} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
    }

    #[test]
    fn classify_valid_auth_request_returns_callback_url() {
        let head = auth_head("?token=jwt&state=deadbeef");
        let outcome = classify_request(&head, "deadbeef", 53824);
        assert_eq!(
            outcome,
            RequestOutcome::AuthCallback {
                callback_url: "http://127.0.0.1:53824/auth?token=jwt&state=deadbeef".to_string()
            }
        );
    }

    #[test]
    fn classify_wrong_state_returns_state_mismatch() {
        let head = auth_head("?token=jwt&state=wrong");
        assert_eq!(
            classify_request(&head, "correct", 53824),
            RequestOutcome::StateMismatch
        );
    }

    #[test]
    fn classify_missing_state_returns_state_mismatch() {
        let head = auth_head("?token=jwt");
        assert_eq!(
            classify_request(&head, "expected", 53824),
            RequestOutcome::StateMismatch
        );
    }

    #[test]
    fn classify_no_query_string_on_auth_path_returns_state_mismatch() {
        let head = "GET /auth HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(
            classify_request(head, "nonce", 53824),
            RequestOutcome::StateMismatch
        );
    }

    #[test]
    fn classify_favicon_returns_not_found() {
        let head = "GET /favicon.ico HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(
            classify_request(head, "state", 53824),
            RequestOutcome::NotFound
        );
    }

    #[test]
    fn classify_root_path_returns_not_found() {
        let head = "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(
            classify_request(head, "state", 53824),
            RequestOutcome::NotFound
        );
    }

    #[test]
    fn classify_post_method_returns_method_not_allowed() {
        let head = "POST /auth?state=abc HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(
            classify_request(head, "abc", 53824),
            RequestOutcome::MethodNotAllowed
        );
    }

    #[test]
    fn classify_callback_url_uses_bound_port() {
        let head = auth_head("?state=s&token=t");
        let outcome = classify_request(&head, "s", 12345);
        assert_eq!(
            outcome,
            RequestOutcome::AuthCallback {
                callback_url: "http://127.0.0.1:12345/auth?state=s&token=t".to_string()
            }
        );
    }

    #[test]
    fn classify_state_only_query_returns_callback() {
        // Minimal valid request: only state param, no other query params.
        let head = auth_head("?state=abc123");
        assert_eq!(
            classify_request(&head, "abc123", 53824),
            RequestOutcome::AuthCallback {
                callback_url: "http://127.0.0.1:53824/auth?state=abc123".to_string()
            }
        );
    }

    // ── bind_loopback (integration: real OS socket) ─────────────────────────

    #[tokio::test]
    async fn bind_loopback_succeeds_on_ephemeral_port() {
        let listener = bind_loopback(0).expect("bind on port 0 must succeed");
        let addr = listener.local_addr().expect("must have local addr");
        assert!(addr.ip().is_loopback());
        assert_ne!(addr.port(), 0, "OS should assign a non-zero ephemeral port");
    }

    #[tokio::test]
    async fn bind_loopback_allows_rebind_via_so_reuseaddr() {
        // Bind once, drop the listener, then bind again on the same port. The
        // short TIME_WAIT window should not block the rebind because we set
        // SO_REUSEADDR.
        let listener = bind_loopback(0).expect("first bind");
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let _ = bind_loopback(port).expect("rebind on same port must succeed with SO_REUSEADDR");
    }
}
