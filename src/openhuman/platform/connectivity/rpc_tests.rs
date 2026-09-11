use super::*;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Serialize env-var mutation across the three `resolve_listen_port_*`
/// tests so they don't race each other under Rust's default parallel
/// runner. Process-global env state means one test's restore can land
/// in another test's read window without this. Same pattern used in
/// `tools/impl/system/lsp.rs`.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn reserve_port() -> std::net::TcpListener {
    std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral test port")
}

async fn spawn_openhuman_probe_listener(
    port: u16,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("bind probe listener");
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok((mut stream, _addr)) = accepted else {
                        break;
                    };
                    let mut req_buf = [0u8; 1024];
                    let _ = stream.read(&mut req_buf).await;
                    let body = r#"{"name":"openhuman","ok":true}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                }
            }
        }
    });

    (task, shutdown_tx)
}

#[tokio::test]
async fn pick_listen_port_preferred_free() {
    let holder = reserve_port();
    let preferred = holder.local_addr().expect("preferred local addr").port();
    drop(holder);

    let result = pick_listen_port_with_policy(
        "127.0.0.1",
        preferred,
        &[],
        RetryPolicy {
            attempts: 0,
            backoff: Duration::from_millis(1),
        },
    )
    .await
    .expect("preferred bind should succeed");

    assert_eq!(result.port, preferred);
    assert_eq!(result.fallback_from, None);
}

#[tokio::test]
async fn pick_listen_port_openhuman_listener_requests_takeover() {
    let holder = reserve_port();
    let preferred = holder.local_addr().expect("preferred local addr").port();
    drop(holder);

    let (server_task, shutdown_tx) = spawn_openhuman_probe_listener(preferred).await;

    let result = pick_listen_port_with_policy(
        "127.0.0.1",
        preferred,
        &[],
        RetryPolicy {
            attempts: 1,
            backoff: Duration::from_millis(10),
        },
    )
    .await;

    let err = result.expect_err("openhuman listener should trigger takeover");
    assert!(
        matches!(err, PickListenPortError::WouldTakeOver { preferred: p, .. } if p == preferred),
        "expected WouldTakeOver for preferred port, got: {err:?}"
    );

    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}

#[tokio::test]
async fn pick_listen_port_other_listener_falls_back() {
    let preferred_listener = reserve_port();
    let preferred = preferred_listener
        .local_addr()
        .expect("preferred local addr")
        .port();
    let busy_fallback_listener = reserve_port();
    let busy_fallback = busy_fallback_listener
        .local_addr()
        .expect("busy fallback local addr")
        .port();
    let free_fallback_holder = reserve_port();
    let free_fallback = free_fallback_holder
        .local_addr()
        .expect("free fallback local addr")
        .port();
    drop(free_fallback_holder);

    let result = pick_listen_port_with_policy(
        "127.0.0.1",
        preferred,
        &[busy_fallback, free_fallback],
        RetryPolicy {
            attempts: 1,
            backoff: Duration::from_millis(10),
        },
    )
    .await
    .expect("fallback bind should succeed");

    assert_eq!(result.port, free_fallback);
    assert_eq!(result.fallback_from, Some(preferred));
}

#[tokio::test]
async fn pick_listen_port_all_candidates_busy_errors() {
    let preferred_listener = reserve_port();
    let preferred = preferred_listener
        .local_addr()
        .expect("preferred local addr")
        .port();
    let fallback1_listener = reserve_port();
    let fallback1 = fallback1_listener
        .local_addr()
        .expect("fallback1 local addr")
        .port();
    let fallback2_listener = reserve_port();
    let fallback2 = fallback2_listener
        .local_addr()
        .expect("fallback2 local addr")
        .port();

    let result = pick_listen_port_with_policy(
        "127.0.0.1",
        preferred,
        &[fallback1, fallback2],
        RetryPolicy {
            attempts: 1,
            backoff: Duration::from_millis(10),
        },
    )
    .await;

    let err = result.expect_err("all-busy path should fail");
    assert!(
        matches!(err, PickListenPortError::NoAvailablePort { preferred: p, ref attempted, .. } if p == preferred && attempted == &vec![fallback1, fallback2]),
        "expected NoAvailablePort with attempted fallback list, got: {err:?}"
    );
}

#[tokio::test]
async fn pick_listen_port_retries_transient_addr_in_use() {
    let preferred_listener = reserve_port();
    let preferred = preferred_listener
        .local_addr()
        .expect("preferred local addr")
        .port();
    let release_task = tokio::spawn(async move {
        sleep(Duration::from_millis(25)).await;
        drop(preferred_listener);
    });

    let result = pick_listen_port_with_policy(
        "127.0.0.1",
        preferred,
        &[],
        RetryPolicy {
            attempts: 6,
            backoff: Duration::from_millis(10),
        },
    )
    .await
    .expect("transient in-use should recover to preferred port");

    release_task.await.expect("release task");
    assert_eq!(result.port, preferred);
    assert_eq!(result.fallback_from, None);
}

// ── is_port_excluded_bind_error (Sentry OPENHUMAN-TAURI-500) ─────────────

#[test]
fn port_excluded_error_matches_wsaeacces_raw_code() {
    // WSAEACCES (os error 10013) — the Windows port-exclusion code from
    // the Sentry event. Must classify as "try a different port" even on
    // non-Windows runners, where 10013 has no special ErrorKind, because
    // we match on the raw code directly.
    let err = std::io::Error::from_raw_os_error(10013);
    assert!(
        is_port_excluded_bind_error(&err),
        "WSAEACCES (10013) must route to the fallback ports"
    );
}

#[test]
fn port_excluded_error_matches_permission_denied_kind() {
    let err = std::io::Error::new(ErrorKind::PermissionDenied, "access denied");
    assert!(
        is_port_excluded_bind_error(&err),
        "PermissionDenied kind must route to the fallback ports"
    );
}

#[test]
fn port_excluded_error_rejects_addr_in_use_and_others() {
    // AddrInUse has its own takeover path and must NOT be treated as an
    // OS exclusion. Unrelated kinds (and unrelated raw codes) must fall
    // through to the existing BindFailed arm so genuine bind bugs surface.
    for err in [
        std::io::Error::new(ErrorKind::AddrInUse, "in use"),
        std::io::Error::new(ErrorKind::ConnectionRefused, "refused"),
        std::io::Error::from_raw_os_error(5), // EIO on unix / not WSAEACCES
    ] {
        assert!(
            !is_port_excluded_bind_error(&err),
            "non-exclusion error must not route to fallback: {err:?}"
        );
    }
}

// ── pick_fallback_port (the path WSAEACCES routes into) ──────────────────

#[tokio::test]
async fn pick_fallback_port_binds_first_free_candidate() {
    // Simulates the post-classification path: the preferred port was
    // unusable (e.g. WSAEACCES), so we try the fallbacks. A free fallback
    // must bind and report `fallback_from: Some(preferred)`.
    let preferred_holder = reserve_port();
    let preferred = preferred_holder.local_addr().unwrap().port();
    let busy_holder = reserve_port();
    let busy = busy_holder.local_addr().unwrap().port();
    let free_holder = reserve_port();
    let free = free_holder.local_addr().unwrap().port();
    drop(free_holder);

    let result = pick_fallback_port(
        "127.0.0.1",
        preferred,
        &[busy, free],
        RetryPolicy {
            attempts: 1,
            backoff: Duration::from_millis(10),
        },
        "port excluded by OS (simulated WSAEACCES)".to_string(),
    )
    .await
    .expect("a free fallback must bind");

    assert_eq!(result.port, free);
    assert_eq!(result.fallback_from, Some(preferred));
}

#[tokio::test]
async fn pick_fallback_port_all_busy_reports_label() {
    // When every fallback is occupied, NoAvailablePort must carry the
    // unusable label (here the OS-exclusion reason) so the diagnostic
    // surface explains *why* the preferred port was skipped.
    let preferred_holder = reserve_port();
    let preferred = preferred_holder.local_addr().unwrap().port();
    let f1_holder = reserve_port();
    let f1 = f1_holder.local_addr().unwrap().port();
    let f2_holder = reserve_port();
    let f2 = f2_holder.local_addr().unwrap().port();

    let err = pick_fallback_port(
        "127.0.0.1",
        preferred,
        &[f1, f2],
        RetryPolicy {
            attempts: 1,
            backoff: Duration::from_millis(10),
        },
        "port excluded by OS (simulated WSAEACCES)".to_string(),
    )
    .await
    .expect_err("all-busy fallbacks must fail");

    assert!(
        matches!(
            err,
            PickListenPortError::NoAvailablePort { preferred: p, ref fingerprint, ref attempted }
                if p == preferred
                    && attempted == &vec![f1, f2]
                    && fingerprint.contains("excluded by OS")
        ),
        "expected NoAvailablePort carrying the exclusion label, got: {err:?}"
    );
}

#[test]
fn snapshot_socket_state_is_uninitialized_without_manager() {
    // The global SocketManager OnceLock may already be set if other
    // tests in this binary installed it. Skip in that case rather than
    // fail; we already cover the live path implicitly.
    if global_socket_manager().is_some() {
        eprintln!(
            "[connectivity::rpc tests] global socket manager installed — \
             skipping uninitialized-state assertion"
        );
        return;
    }
    let (state, err) = snapshot_socket_state();
    assert_eq!(state, "uninitialized");
    assert!(err.is_none());
}

#[test]
fn resolve_listen_port_defaults_to_7788_when_env_unset() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    // Use a UUID-ish guard so we don't clobber an env the test runner
    // genuinely needs. SAFETY: env mutation is process-global; we
    // restore at the end. See SAFETY note in `cargo test --doc`.
    let prev_port = std::env::var("OPENHUMAN_CORE_PORT").ok();
    // resolve_listen_port() also reads OPENHUMAN_CORE_RPC_URL ahead of
    // OPENHUMAN_CORE_PORT, so an inherited URL from the runner would
    // make this assertion nondeterministic. Save + clear both.
    let prev_url = std::env::var("OPENHUMAN_CORE_RPC_URL").ok();
    // SAFETY: standard Rust test pattern — env access is unsafe in 2024
    // edition because it isn't thread-safe. Tests are single-threaded
    // for this scope and we restore in the same body.
    unsafe {
        std::env::remove_var("OPENHUMAN_CORE_PORT");
        std::env::remove_var("OPENHUMAN_CORE_RPC_URL");
    }
    assert_eq!(resolve_listen_port(), DEFAULT_CORE_PORT);
    if let Some(value) = prev_port {
        unsafe {
            std::env::set_var("OPENHUMAN_CORE_PORT", value);
        }
    }
    if let Some(value) = prev_url {
        unsafe {
            std::env::set_var("OPENHUMAN_CORE_RPC_URL", value);
        }
    }
}

#[test]
fn resolve_listen_port_honours_env_override() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let prev_port = std::env::var("OPENHUMAN_CORE_PORT").ok();
    let prev_url = std::env::var("OPENHUMAN_CORE_RPC_URL").ok();
    unsafe {
        // Clear OPENHUMAN_CORE_RPC_URL so OPENHUMAN_CORE_PORT is the
        // resolved value (URL has higher priority in resolve_listen_port).
        std::env::remove_var("OPENHUMAN_CORE_RPC_URL");
        std::env::set_var("OPENHUMAN_CORE_PORT", "65000");
    }
    assert_eq!(resolve_listen_port(), 65000);
    match prev_port {
        Some(value) => unsafe { std::env::set_var("OPENHUMAN_CORE_PORT", value) },
        None => unsafe { std::env::remove_var("OPENHUMAN_CORE_PORT") },
    }
    match prev_url {
        Some(value) => unsafe { std::env::set_var("OPENHUMAN_CORE_RPC_URL", value) },
        None => unsafe { std::env::remove_var("OPENHUMAN_CORE_RPC_URL") },
    }
}

#[test]
fn resolve_listen_port_falls_back_on_invalid_env() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let prev_port = std::env::var("OPENHUMAN_CORE_PORT").ok();
    let prev_url = std::env::var("OPENHUMAN_CORE_RPC_URL").ok();
    unsafe {
        std::env::remove_var("OPENHUMAN_CORE_RPC_URL");
        std::env::set_var("OPENHUMAN_CORE_PORT", "not-a-number");
    }
    assert_eq!(resolve_listen_port(), DEFAULT_CORE_PORT);
    match prev_port {
        Some(value) => unsafe { std::env::set_var("OPENHUMAN_CORE_PORT", value) },
        None => unsafe { std::env::remove_var("OPENHUMAN_CORE_PORT") },
    }
    match prev_url {
        Some(value) => unsafe { std::env::set_var("OPENHUMAN_CORE_RPC_URL", value) },
        None => unsafe { std::env::remove_var("OPENHUMAN_CORE_RPC_URL") },
    }
}

#[test]
fn resolve_listen_port_prefers_openhuman_core_rpc_url() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let prev_rpc = std::env::var("OPENHUMAN_CORE_RPC_URL").ok();
    let prev_port = std::env::var("OPENHUMAN_CORE_PORT").ok();
    unsafe {
        std::env::set_var("OPENHUMAN_CORE_RPC_URL", "http://127.0.0.1:7794/rpc");
        std::env::set_var("OPENHUMAN_CORE_PORT", "7788");
    }
    assert_eq!(resolve_listen_port(), 7794);
    match prev_rpc {
        Some(value) => unsafe { std::env::set_var("OPENHUMAN_CORE_RPC_URL", value) },
        None => unsafe { std::env::remove_var("OPENHUMAN_CORE_RPC_URL") },
    }
    match prev_port {
        Some(value) => unsafe { std::env::set_var("OPENHUMAN_CORE_PORT", value) },
        None => unsafe { std::env::remove_var("OPENHUMAN_CORE_PORT") },
    }
}

#[test]
fn snapshot_populates_all_fields() {
    let snap = snapshot();
    // Don't assert exact pid; just that we set one.
    assert!(snap.sidecar_pid.is_some(), "sidecar_pid should be set");
    assert!(snap.listen_port > 0, "listen_port should be non-zero");
    assert!(
        !snap.socket_state.is_empty(),
        "socket_state should be non-empty"
    );
}

#[tokio::test]
async fn diag_returns_serializable_payload() {
    let outcome = diag().await.expect("diag rpc");
    let json = outcome
        .into_cli_compatible_json()
        .expect("into_cli_compatible_json");
    assert!(json.is_object(), "payload should be a JSON object");
    // `single_log` adds a log entry, so `into_cli_compatible_json` wraps
    // the value inside `{ "result": ..., "logs": [...] }`. Look for the
    // diag payload under `result`.
    let result = json.get("result").expect("result envelope key present");
    let diag = result.get("diag").expect("diag key present under result");
    assert!(diag.get("socket_state").is_some());
    assert!(diag.get("listen_port").is_some());
    assert!(diag.get("listen_port_in_use").is_some());
}
