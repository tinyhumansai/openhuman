use super::{
    current_rpc_token, default_core_port, generate_rpc_token, is_expected_port_clash,
    is_openhuman_root_body, parse_lsof_pid, parse_netstat_pid, CoreProcessHandle,
};
use std::sync::{Mutex, MutexGuard, OnceLock};

fn env_lock() -> MutexGuard<'static, ()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    // Recover from poison: when one test panics while holding this lock
    // (e.g. an embedded-core readiness timeout under CI load), every
    // subsequent test in the suite would otherwise cascade-fail with
    // "env lock poisoned" — turning one real flake into three. The lock
    // only serializes process-wide env-var mutation; the inner `()`
    // carries no state that poisoning could corrupt.
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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

#[test]
fn parse_netstat_pid_skips_protected_kernel_pids() {
    // HTTP.sys / driver-level reservations occasionally show as LISTENING
    // under PID 4 (NT Kernel) or PID 0 (System Idle). Returning those pids
    // would lead startup recovery to call taskkill on a process that cannot
    // be signalled from user mode — aborting the entire takeover flow.
    // The parser must treat these entries as "no owner" so callers fall
    // back to the port-reroute path instead of trying to kill the kernel.
    let stdout = "\
Active Connections

  Proto  Local Address          Foreign Address        State           PID
  TCP    127.0.0.1:7788         0.0.0.0:0              LISTENING       4
  TCP    127.0.0.1:7789         0.0.0.0:0              LISTENING       0
  TCP    127.0.0.1:7790         0.0.0.0:0              LISTENING       1234
";
    assert_eq!(parse_netstat_pid(stdout, 7788), None);
    assert_eq!(parse_netstat_pid(stdout, 7789), None);
    assert_eq!(parse_netstat_pid(stdout, 7790), Some(1234));
}

#[test]
fn parse_netstat_pid_falls_through_protected_to_real_owner_on_dual_stack() {
    // Real-world dual-stack listener: kernel-reserved entry sits ahead of
    // the actual user-mode owner on the same port. The parser must keep
    // scanning past the protected pid and return the genuine owner.
    let stdout = "\
  Proto  Local Address          Foreign Address        State           PID
  TCP    [::]:7788              [::]:0                 LISTENING       4
  TCP    127.0.0.1:7788         0.0.0.0:0              LISTENING       9999
";
    assert_eq!(parse_netstat_pid(stdout, 7788), Some(9999));
}

// ---------------------------------------------------------------------------
// Windows end-to-end port-takeover test
//
// Spawns a real child process that occupies a TCP port, then walks the same
// path the Tauri host walks at startup (find_pid_on_port → kill_pid_force →
// is_port_open) and asserts the port is actually freed. This is the
// behavior the user reported broken — a unit-only parser test is not enough
// to catch netstat/taskkill drift on real Windows machines.
// ---------------------------------------------------------------------------

#[cfg(windows)]
#[test]
fn windows_port_takeover_finds_and_kills_listener() {
    use crate::process_kill::kill_pid_force;
    use std::net::TcpListener;
    use std::os::windows::process::CommandExt;
    use std::time::{Duration, Instant};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // Bind in this process first to claim an ephemeral free port the OS
    // picks for us, capture the port, then drop the listener so the child
    // can bind to the same port. There is a tiny TOCTOU window here but
    // ephemeral ports on Windows are not aggressively recycled so it is
    // robust enough for a single-shot test.
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let port = probe.local_addr().expect("probe addr").port();
    drop(probe);

    // Use PowerShell to spawn a listener that holds the port open for 60s.
    // PowerShell ships with every supported Windows version.
    let script = format!(
        "$l = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, {port}); \
         $l.Start(); Start-Sleep -Seconds 60; $l.Stop()"
    );
    let mut child = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .spawn()
        .expect("spawn powershell listener");

    // Wait until the listener is actually bound (PowerShell startup is slow).
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut bound = false;
    while Instant::now() < deadline {
        if std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().unwrap(),
            Duration::from_millis(100),
        )
        .is_ok()
        {
            bound = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !bound {
        let _ = child.kill();
        let _ = child.wait();
        panic!("child listener never bound to 127.0.0.1:{port}");
    }

    // Walk the production path: pid lookup via netstat, then force-kill.
    let pid = match super::find_pid_on_port(port) {
        Some(pid) => pid,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("find_pid_on_port returned None for port {port}");
        }
    };
    // The pid we discovered won't be `child.id()` directly — the powershell
    // process is the listener, and on Windows `child.id()` IS that pid.
    // Sanity-check they match so a future netstat parser regression is loud.
    // Tear down the child *before* panicking so a 60s listener doesn't leak
    // into the rest of the test suite.
    if pid != child.id() {
        let expected = child.id();
        let _ = child.kill();
        let _ = child.wait();
        panic!("find_pid_on_port returned pid {pid}, expected child pid {expected}");
    }

    kill_pid_force(pid).expect("force-kill listener");

    // Verify the port is actually free within a reasonable window — this is
    // the assertion that fails when taskkill mis-reports success or when
    // /T fails to take down the powershell subtree.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut freed = false;
    while Instant::now() < deadline {
        if std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().unwrap(),
            Duration::from_millis(100),
        )
        .is_err()
        {
            freed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let _ = child.wait();
    assert!(
        freed,
        "port {port} still bound after kill_pid_force(pid={pid})"
    );

    // Idempotency: kill the same pid again — must be Ok, not Err, because
    // the process is already gone and recovery code calls force-kill after
    // a re-validation that may race.
    kill_pid_force(pid).expect("kill_pid_force on dead pid must be idempotent");
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

#[test]
fn startup_timeout_cleanup_aborts_task_and_clears_slot() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let handle = CoreProcessHandle::new(19006);
        let task = tokio::spawn(async {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            Ok::<(), anyhow::Error>(())
        });

        {
            let mut guard = handle.task.lock().await;
            *guard = Some(task);
        }

        let message = handle.cleanup_startup_timeout(false, false).await;

        assert!(
            message.contains("core process did not become ready within"),
            "timeout message should include the readiness budget: {message}"
        );
        assert!(
            message.contains("ready_signal=false"),
            "timeout message should include ready signal state: {message}"
        );
        assert!(
            message.contains("port_open=false"),
            "timeout message should include final port state: {message}"
        );
        assert!(
            message.contains("task_state=running"),
            "timeout message should include task state: {message}"
        );
        assert!(
            handle.task.lock().await.is_none(),
            "cleanup must clear the managed task slot so retry can spawn fresh"
        );
        assert!(
            handle.shutdown_token_is_cancelled().await,
            "cleanup must cancel the startup token before aborting"
        );
    });
}
