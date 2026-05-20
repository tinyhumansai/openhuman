use super::{
    current_rpc_token, default_core_port, generate_rpc_token, is_expected_port_clash,
    is_openhuman_root_body, parse_lsof_pid, parse_netstat_pid, CoreProcessHandle,
};
use std::sync::{Mutex, MutexGuard, OnceLock};

fn env_lock() -> MutexGuard<'static, ()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock poisoned")
}

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

#[test]
fn default_core_port_env_and_fallback() {
    let _env_lock = env_lock();
    let _unset = EnvGuard::unset("OPENHUMAN_CORE_PORT");
    assert_eq!(default_core_port(), 7788);

    let _set = EnvGuard::set("OPENHUMAN_CORE_PORT", "8899");
    assert_eq!(default_core_port(), 8899);
}

#[test]
fn core_process_handle_new_creates_instance() {
    let handle = CoreProcessHandle::new(9999);
    assert_eq!(handle.port(), 9999);
    assert_eq!(handle.rpc_url(), "http://127.0.0.1:9999/rpc");
}

#[test]
fn ready_signal_updates_runtime_port_and_fallback_notice() {
    let handle = CoreProcessHandle::new(7788);
    handle.apply_embedded_ready_signal(openhuman_core::core::jsonrpc::EmbeddedReadySignal {
        port: 7789,
        fallback_from: Some(7788),
    });
    assert_eq!(handle.port(), 7789);
    assert_eq!(handle.rpc_url(), "http://127.0.0.1:7789/rpc");
    let notice = handle
        .take_last_port_fallback_notice()
        .expect("fallback notice should be present");
    assert_eq!(notice.preferred_port, 7788);
    assert_eq!(notice.chosen_port, 7789);
    assert!(
        handle.take_last_port_fallback_notice().is_none(),
        "fallback notice should be consumed once"
    );
}

/// Issue #1613: when the preferred port is occupied by a non-OpenHuman
/// listener, startup should fall back to a nearby port instead of failing.
#[test]
fn ensure_running_falls_back_for_unknown_listener_on_port() {
    let _env_lock = env_lock();
    let _unset = EnvGuard::unset("OPENHUMAN_CORE_REUSE_EXISTING");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let (result, chosen_port, notice) = rt.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let port = listener.local_addr().expect("local addr").port();
        let handle = CoreProcessHandle::new(port);
        let result = handle.ensure_running().await;
        let chosen_port = handle.port();
        let notice = handle.take_last_port_fallback_notice();
        handle.shutdown().await;
        (result, chosen_port, notice)
    });
    assert!(
        result.is_ok(),
        "ensure_running should recover via fallback when preferred port is occupied: {result:?}"
    );
    assert!(
        notice.is_some(),
        "fallback notice should be set when preferred port is occupied"
    );
    let notice = notice.expect("notice set");
    assert_ne!(
        chosen_port, notice.preferred_port,
        "fallback must choose a different port"
    );
    assert_eq!(
        chosen_port, notice.chosen_port,
        "chosen port should match fallback notice payload"
    );
}

#[test]
fn ensure_running_falls_back_to_7789_when_7788_is_busy() {
    let _env_lock = env_lock();
    let _unset = EnvGuard::unset("OPENHUMAN_CORE_REUSE_EXISTING");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let listener = match tokio::net::TcpListener::bind("127.0.0.1:7788").await {
            Ok(listener) => listener,
            Err(err) => {
                eprintln!(
                    "[core_process tests] skipping fixed-port fallback test; 7788 unavailable: {err}"
                );
                return;
            }
        };

        let handle = CoreProcessHandle::new(7788);
        let result = handle.ensure_running().await;
        assert!(
            result.is_ok(),
            "ensure_running should recover by binding a fallback port: {result:?}"
        );
        // Accept any port in the configured fallback range 7789..=7798 — a
        // parallel test or environmental squatter on a single fallback port
        // shouldn't fail the broader contract that fallback recovery works.
        let chosen = handle.port();
        assert!(
            (7789..=7798).contains(&chosen),
            "with 7788 occupied, core should bind to a fallback in 7789..=7798, got {chosen}"
        );
        let notice = handle
            .take_last_port_fallback_notice()
            .expect("fallback notice should be present");
        assert_eq!(notice.preferred_port, 7788);
        assert_eq!(
            notice.chosen_port, chosen,
            "fallback notice payload should match the bound port"
        );
        assert!(
            (7789..=7798).contains(&notice.chosen_port),
            "fallback notice chosen_port should be in 7789..=7798, got {}",
            notice.chosen_port
        );
        handle.shutdown().await;
        drop(listener);
    });
}

/// Escape hatch: setting `OPENHUMAN_CORE_REUSE_EXISTING=1` opts back into
/// the legacy attach-to-anything behavior for manual harnesses.
#[test]
fn ensure_running_reuses_unknown_listener_when_override_set() {
    let _env_lock = env_lock();
    let _override = EnvGuard::set("OPENHUMAN_CORE_REUSE_EXISTING", "1");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let result = rt.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let port = listener.local_addr().expect("local addr").port();
        let handle = CoreProcessHandle::new(port);
        handle.ensure_running().await
    });
    assert!(
        result.is_ok(),
        "override should restore legacy fast-path: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Listener fingerprinting (issue #1130)
// ---------------------------------------------------------------------------

#[test]
fn is_openhuman_root_body_matches_canonical_root_response() {
    // Mirrors the JSON shape produced by `core/jsonrpc.rs::root_handler`.
    let body = r#"{
        "name": "openhuman",
        "ok": true,
        "endpoints": {"health": "/health", "rpc": "/rpc"}
    }"#;
    assert!(is_openhuman_root_body(body));
}

#[test]
fn is_openhuman_root_body_rejects_other_services() {
    assert!(!is_openhuman_root_body(r#"{"name": "something-else"}"#));
    assert!(!is_openhuman_root_body(r#"{"ok": true}"#));
    assert!(!is_openhuman_root_body("not json at all"));
    assert!(!is_openhuman_root_body(""));
    // Wrong type for `name`.
    assert!(!is_openhuman_root_body(r#"{"name": 42}"#));
}

#[test]
fn expected_port_clash_classifier_matches_benign_probe_shapes() {
    assert!(is_expected_port_clash(
        "probe GET / failed: error sending request for url (http://127.0.0.1:7788/)"
    ));
    assert!(is_expected_port_clash(
        "probe GET / failed: connection refused"
    ));
    assert!(is_expected_port_clash(
        "probe GET / returned status 404 Not Found"
    ));
    assert!(is_expected_port_clash("probe GET / returned status 200 OK"));
    assert!(is_expected_port_clash(
        "probe GET / body did not identify as openhuman (\"hello\")"
    ));
}

#[test]
fn expected_port_clash_classifier_matches_windows_acl_bind_shapes() {
    assert!(is_expected_port_clash(
        "Failed to bind to 127.0.0.1:7788: access denied (os error 10013)"
    ));
    assert!(is_expected_port_clash(
        "Failed to bind to 127.0.0.1:7788: WSAEACCES"
    ));
}

#[test]
fn expected_port_clash_classifier_rejects_unknown_probe_shapes() {
    assert!(!is_expected_port_clash(
        "probe GET / failed: TLS handshake failed: protocol error"
    ));
    assert!(!is_expected_port_clash(
        "probe GET / body read failed: unexpected eof"
    ));
}

#[test]
fn parse_lsof_pid_picks_first_pid() {
    assert_eq!(parse_lsof_pid("12345\n"), Some(12345));
    // Multiple pids — pick the first non-empty line. lsof can emit several
    // when multiple sockets share the port (IPv4/IPv6).
    assert_eq!(parse_lsof_pid("\n  9876  \n12345\n"), Some(9876));
    assert_eq!(parse_lsof_pid(""), None);
    assert_eq!(parse_lsof_pid("not-a-pid\n"), None);
}

#[test]
fn parse_netstat_pid_finds_listening_entry() {
    // Sample shape from `netstat -ano -p TCP` on Windows.
    let stdout = "\
Active Connections

  Proto  Local Address          Foreign Address        State           PID
  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1024
  TCP    127.0.0.1:7788         0.0.0.0:0              LISTENING       4242
  TCP    127.0.0.1:50000        127.0.0.1:7788         ESTABLISHED     5555
";
    assert_eq!(parse_netstat_pid(stdout, 7788), Some(4242));
    assert_eq!(parse_netstat_pid(stdout, 9999), None);
}

// ---------------------------------------------------------------------------
// Token generation tests
// ---------------------------------------------------------------------------

/// `generate_rpc_token` must produce a 64-character lowercase hex string
/// (32 bytes × 2 hex digits = 64 chars), matching the format expected by the
/// core's auth middleware.
#[test]
fn generate_rpc_token_produces_64_hex_chars() {
    let token = generate_rpc_token();
    assert_eq!(
        token.len(),
        64,
        "256-bit token → 64 hex chars, got {token:?}"
    );
    assert!(
        token.chars().all(|c| c.is_ascii_hexdigit()),
        "token must be hex, got {token:?}"
    );
    assert!(
        token.chars().all(|c| !c.is_uppercase()),
        "token must be lowercase hex, got {token:?}"
    );
}

/// Each call generates a different token (CSPRNG — not a constant).
#[test]
fn generate_rpc_token_is_not_constant() {
    assert_ne!(
        generate_rpc_token(),
        generate_rpc_token(),
        "two consecutive tokens must differ"
    );
}

/// `CoreProcessHandle::new` must produce a non-empty, correctly-formatted
/// bearer token immediately — no file I/O or timing dependency.
#[test]
fn core_process_handle_new_token_is_valid() {
    let handle = CoreProcessHandle::new(19001);
    let token = handle.rpc_token();
    assert_eq!(token.len(), 64, "handle token must be 64 hex chars");
    assert!(
        token.chars().all(|c| c.is_ascii_hexdigit()),
        "handle token must be hex"
    );
}

/// `CoreProcessHandle::new()` must NOT publish the token to the global
/// `CURRENT_RPC_TOKEN`. The global is set only after `ensure_running()`
/// successfully spawns the embedded server with `OPENHUMAN_CORE_TOKEN` in
/// scope. Advertising the token before spawn would 401 against any process
/// already listening on the port that never received this token.
#[test]
fn new_does_not_publish_global_token() {
    let before = current_rpc_token();
    let handle = CoreProcessHandle::new(19002);
    let after = current_rpc_token();

    assert_ne!(
        after.as_deref(),
        Some(handle.rpc_token()),
        "new() must not publish its token to CURRENT_RPC_TOKEN before ensure_running() spawns"
    );
    assert_eq!(
        before, after,
        "new() must leave CURRENT_RPC_TOKEN unchanged"
    );
}

/// Two handles constructed sequentially must each have a unique token.
#[test]
fn each_handle_has_unique_token() {
    let h1 = CoreProcessHandle::new(19003);
    let h2 = CoreProcessHandle::new(19004);

    assert_ne!(
        h1.rpc_token(),
        h2.rpc_token(),
        "each handle must have a unique token"
    );
}

#[test]
fn send_terminate_signal_cancels_shutdown_token() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let handle = CoreProcessHandle::new(19005);
        assert!(!handle.shutdown_token_is_cancelled().await);

        handle.send_terminate_signal().await;

        assert!(
            handle.shutdown_token_is_cancelled().await,
            "send_terminate_signal must cancel graceful Axum shutdown before aborting the task"
        );
    });
}
