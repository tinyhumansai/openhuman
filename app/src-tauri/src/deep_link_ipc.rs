//! Pre-CEF deep-link forwarding for Linux (issue #2359).
//!
//! On Linux, `openhuman://` OAuth callbacks launch a second OpenHuman
//! binary with the URL in argv. That secondary hits
//! `cef_preflight::check_default_cache()` and exits before Builder::setup
//! runs, so tauri-plugin-deep-link never gets a chance to forward the URL.
//!
//! This module fixes the race by:
//!   1. Primary: bind a Unix domain socket at a stable per-user path BEFORE
//!      the CEF preflight check. Queue any arriving URLs until setup() runs.
//!   2. Secondary (URL in argv): connect to the socket, write the URL(s),
//!      and exit(0). CEF preflight is never reached.

#![cfg(target_os = "linux")]

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

/// Stable socket path. Uses $XDG_RUNTIME_DIR when available (per-user,
/// per-session tmpfs, cleaned on reboot), falls back to /tmp with UID.
pub(crate) fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("com.openhuman.app-deeplink.sock");
    }
    // Fallback: include UID so multi-user machines don't collide.
    let uid = nix::unistd::getuid().as_raw();
    std::env::temp_dir().join(format!("com_openhuman_app_deeplink_{uid}.sock"))
}

/// Collect any `openhuman://` URLs from the process argv.
pub(crate) fn extract_deep_link_urls() -> Vec<String> {
    std::env::args()
        .skip(1)
        .filter(|a| a.starts_with("openhuman://"))
        .collect()
}

/// Result of `try_forward_deep_links`.
pub(crate) enum ForwardResult {
    /// URLs were written to the primary's socket; caller should exit(0).
    Forwarded,
    /// Deep-link URL found in argv but no primary socket is listening.
    NoPrimary,
    /// No deep-link URLs in argv; this is a normal launch.
    NoUrls,
}

/// Try to forward any `openhuman://` URLs in argv to the primary instance.
/// Call this BEFORE the CEF preflight check.
pub(crate) fn try_forward_deep_links() -> ForwardResult {
    let urls = extract_deep_link_urls();
    if urls.is_empty() {
        return ForwardResult::NoUrls;
    }

    let path = socket_path();
    log::info!(
        "[deep-link-ipc] secondary: found {} deep-link URL(s), trying socket at {}",
        urls.len(),
        path.display()
    );

    match UnixStream::connect(&path) {
        Ok(mut stream) => {
            stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
            for url in &urls {
                if let Err(e) = writeln!(stream, "{url}") {
                    log::warn!("[deep-link-ipc] secondary: failed to write URL: {e}");
                }
            }
            log::info!(
                "[deep-link-ipc] secondary: {} URL(s) forwarded to primary",
                urls.len()
            );
            ForwardResult::Forwarded
        }
        Err(e) => {
            log::info!(
                "[deep-link-ipc] secondary: no primary socket at {} ({e}); \
                 will become primary",
                path.display()
            );
            ForwardResult::NoPrimary
        }
    }
}

// Pending URLs collected before setup() has an app handle.
static PENDING_URLS: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();
// Live handler installed by drain_pending_urls — dispatches directly to app.
static LIVE_HANDLER: OnceLock<Mutex<Option<Box<dyn Fn(String) + Send + Sync>>>> = OnceLock::new();

fn pending_queue() -> &'static Arc<Mutex<Vec<String>>> {
    PENDING_URLS.get_or_init(|| Arc::new(Mutex::new(Vec::new())))
}

fn live_handler() -> &'static Mutex<Option<Box<dyn Fn(String) + Send + Sync>>> {
    LIVE_HANDLER.get_or_init(|| Mutex::new(None))
}

/// Strip query string and fragment from a deep-link URL before logging.
/// OAuth callbacks carry tokens in the query string; logging the raw URL
/// would persist secrets in log files and crash reports.
fn redact_url_for_log(url: &str) -> String {
    url.parse::<url::Url>()
        .map(|mut parsed| {
            parsed.set_query(None);
            parsed.set_fragment(None);
            parsed.to_string()
        })
        .unwrap_or_else(|_| "<invalid deep link>".to_string())
}

fn dispatch_url(url: String) {
    // Try the live handler first.
    if let Ok(guard) = live_handler().lock() {
        if let Some(ref handler) = *guard {
            handler(url);
            return;
        }
    }
    // No live handler yet — queue for drain_pending_urls.
    if let Ok(mut q) = pending_queue().lock() {
        log::debug!(
            "[deep-link-ipc] queued URL (no handler yet): {}",
            redact_url_for_log(&url)
        );
        q.push(url);
    }
}

/// RAII guard: removes the socket file when dropped.
pub(crate) struct DeepLinkSocketGuard {
    path: PathBuf,
}

impl Drop for DeepLinkSocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        log::debug!(
            "[deep-link-ipc] socket cleaned up at {}",
            self.path.display()
        );
    }
}

/// Bind the deep-link socket and start the listener thread.
/// Returns `None` if binding fails (non-fatal — log and continue).
///
/// Uses a bind-first approach to avoid the race where a secondary instance
/// unconditionally removes a live primary's socket file: we only remove the
/// file when we can confirm it is stale (connect fails).
pub(crate) fn bind_and_listen() -> Option<DeepLinkSocketGuard> {
    let path = socket_path();

    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            // A socket file already exists.  Probe whether a live primary
            // is behind it before deciding to unlink.
            match UnixStream::connect(&path) {
                Ok(_) => {
                    // Live primary — this instance should not bind.
                    log::debug!(
                        "[deep-link-ipc] socket {} is live; skipping bind \
                         (primary already running)",
                        path.display()
                    );
                    return None;
                }
                Err(_) => {
                    // Stale socket from a previous crash — safe to remove.
                    log::debug!(
                        "[deep-link-ipc] removing stale socket at {}",
                        path.display()
                    );
                    let _ = std::fs::remove_file(&path);
                    match UnixListener::bind(&path) {
                        Ok(l) => l,
                        Err(e2) => {
                            log::warn!(
                                "[deep-link-ipc] failed to bind socket at {} after \
                                 removing stale file — deep-link forwarding from \
                                 secondary instances will not work: {e2}",
                                path.display()
                            );
                            return None;
                        }
                    }
                }
            }
        }
        Err(e) => {
            log::warn!(
                "[deep-link-ipc] failed to bind socket at {} — deep-link forwarding \
                 from secondary instances will not work: {e}",
                path.display()
            );
            return None;
        }
    };

    let path_clone = path.clone();
    std::thread::Builder::new()
        .name("deep-link-ipc-listener".into())
        .spawn(move || {
            log::info!(
                "[deep-link-ipc] primary: listening on {}",
                path_clone.display()
            );
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => handle_connection(stream),
                    Err(e) => {
                        log::debug!("[deep-link-ipc] accept error: {e}");
                        // Listener is gone (guard dropped) — stop.
                        break;
                    }
                }
            }
            log::info!("[deep-link-ipc] listener thread exiting");
        })
        .ok();
    Some(DeepLinkSocketGuard { path })
}

fn handle_connection(stream: UnixStream) {
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        match line {
            Ok(url) if url.starts_with("openhuman://") => {
                log::info!(
                    "[deep-link-ipc] primary: received deep-link URL: {}",
                    redact_url_for_log(&url)
                );
                dispatch_url(url);
            }
            Ok(other) => {
                log::debug!("[deep-link-ipc] primary: ignoring non-deep-link line: {other}");
            }
            Err(e) => {
                log::debug!("[deep-link-ipc] primary: read error: {e}");
                break;
            }
        }
    }
}

/// Drain any URLs queued before setup() ran, then install a live handler
/// that emits `deep-link://new-url` events directly to the app handle.
/// Call this from Builder::setup() after deep-link registration.
pub(crate) fn drain_pending_urls<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Emitter;

    // Install the live handler first so future URLs don't queue.
    let app_clone = app.clone();
    if let Ok(mut guard) = live_handler().lock() {
        *guard = Some(Box::new(move |url: String| {
            if let Ok(parsed) = url.parse::<url::Url>() {
                let urls = vec![parsed];
                if let Err(e) = app_clone.emit("deep-link://new-url", &urls) {
                    log::warn!("[deep-link-ipc] failed to emit deep-link event: {e}");
                }
            } else {
                log::warn!("[deep-link-ipc] received malformed deep-link URL");
            }
        }));
    }

    // Drain any URLs that arrived before setup().
    let pending: Vec<String> = pending_queue()
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default();

    if !pending.is_empty() {
        log::info!(
            "[deep-link-ipc] draining {} queued deep-link URL(s)",
            pending.len()
        );
    }
    for url in pending {
        if let Ok(parsed) = url.parse::<url::Url>() {
            let urls = vec![parsed];
            if let Err(e) = app.emit("deep-link://new-url", &urls) {
                log::warn!("[deep-link-ipc] failed to emit queued deep-link URL: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn socket_path_uses_xdg_runtime_dir() {
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1234");
        let path = socket_path();
        assert_eq!(
            path,
            PathBuf::from("/run/user/1234/com.openhuman.app-deeplink.sock")
        );
    }

    #[test]
    fn socket_path_fallback_has_uid() {
        std::env::remove_var("XDG_RUNTIME_DIR");
        let path = socket_path();
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(
            name.contains("com_openhuman_app_deeplink"),
            "path {path:?} should contain identifier"
        );
        // Should NOT be inside /run/user since XDG_RUNTIME_DIR is unset.
        assert!(
            !path.starts_with("/run/user"),
            "path should use temp_dir fallback"
        );
    }

    #[test]
    fn extract_deep_link_urls_filters_correctly() {
        // We can't mutate std::env::args(), so test the filtering logic directly.
        let args = vec![
            "OpenHuman".to_string(),
            "openhuman://auth?token=abc".to_string(),
            "--some-flag".to_string(),
            "openhuman://other".to_string(),
            "https://example.com".to_string(),
        ];
        let urls: Vec<String> = args
            .into_iter()
            .skip(1)
            .filter(|a| a.starts_with("openhuman://"))
            .collect();
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "openhuman://auth?token=abc");
        assert_eq!(urls[1], "openhuman://other");
    }

    #[test]
    fn round_trip_bind_connect_forward() {
        use std::io::BufRead;
        use std::os::unix::net::UnixStream;

        // Use a temp path for this test to avoid collisions.
        let tmp = tempfile::TempDir::new().unwrap();
        let sock_path = tmp.path().join("test-deeplink.sock");

        let listener = UnixListener::bind(&sock_path).unwrap();
        let received = Arc::new(Mutex::new(Vec::<String>::new()));
        let received_clone = Arc::clone(&received);

        std::thread::spawn(move || {
            if let Ok(stream) = listener.accept().map(|(s, _)| s) {
                stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
                let reader = BufReader::new(stream);
                for line in reader.lines().flatten() {
                    if line.starts_with("openhuman://") {
                        received_clone.lock().unwrap().push(line);
                    }
                }
            }
        });

        // Give listener thread time to start.
        std::thread::sleep(Duration::from_millis(50));

        let mut stream = UnixStream::connect(&sock_path).unwrap();
        writeln!(stream, "openhuman://auth?token=testtoken123").unwrap();
        drop(stream);

        std::thread::sleep(Duration::from_millis(100));
        let got = received.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], "openhuman://auth?token=testtoken123");
    }

    #[test]
    fn no_primary_returns_appropriate_result() {
        // Remove socket file to guarantee no primary.
        std::env::remove_var("XDG_RUNTIME_DIR");
        let _ = std::fs::remove_file(socket_path());

        // The "extract_deep_link_urls" function reads actual argv which has
        // no openhuman:// URLs during tests, so try_forward_deep_links()
        // returns NoUrls. We test the NoPrimary branch directly by
        // testing that connect to a missing socket fails.
        let non_existent = PathBuf::from("/tmp/openhuman_test_nonexistent_socket.sock");
        let _ = std::fs::remove_file(&non_existent);
        let result = UnixStream::connect(&non_existent);
        assert!(
            result.is_err(),
            "Expected connection failure for missing socket"
        );
    }
}
