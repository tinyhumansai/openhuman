use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn reserve_port() -> std::net::TcpListener {
    std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral test port")
}

async fn spawn_openhuman_probe_listener(
    listener: std::net::TcpListener,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    listener
        .set_nonblocking(true)
        .expect("set probe listener non-blocking");
    let listener =
        tokio::net::TcpListener::from_std(listener).expect("adopt reserved probe listener");
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
        OccupiedByCore::Takeover,
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

    let (server_task, shutdown_tx) = spawn_openhuman_probe_listener(holder).await;

    let result = pick_listen_port_with_policy(
        "127.0.0.1",
        preferred,
        &[],
        RetryPolicy {
            attempts: 1,
            backoff: Duration::from_millis(10),
        },
        OccupiedByCore::Takeover,
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
async fn pick_listen_port_openhuman_listener_falls_back_when_asked() {
    let holder = reserve_port();
    let preferred = holder.local_addr().expect("preferred local addr").port();
    // Use the same neighbouring-port candidates as production instead of
    // sampling and releasing one ephemeral port. Keep the OS reservation
    // until the probe listener adopts it so other test processes cannot race
    // the bind.
    let fallbacks: Vec<u16> = (1..=10)
        .filter_map(|delta| preferred.checked_add(delta))
        .collect();

    let (server_task, shutdown_tx) = spawn_openhuman_probe_listener(holder).await;

    let picked = pick_listen_port_with_policy(
        "127.0.0.1",
        preferred,
        &fallbacks,
        RetryPolicy {
            attempts: 1,
            backoff: Duration::from_millis(10),
        },
        OccupiedByCore::Fallback,
    )
    .await
    .expect("a live neighbouring core should not block a headless core");
    assert!(fallbacks.contains(&picked.port));
    assert_eq!(picked.fallback_from, Some(preferred));

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
        OccupiedByCore::Takeover,
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
        OccupiedByCore::Takeover,
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
        OccupiedByCore::Takeover,
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
        // Keep this raw code outside the platform-specific errno/Win32
        // ranges used by the classifier (5 is PermissionDenied on Windows).
        std::io::Error::from_raw_os_error(12345),
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

#[tokio::test]
async fn pick_listen_port_identifies_ipv6_openhuman_listener_when_supported() {
    let Ok(listener) = tokio::net::TcpListener::bind("[::1]:0").await else {
        return; // IPv6 loopback is optional on CI hosts.
    };
    let port = listener.local_addr().expect("IPv6 probe address").port();
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok((mut stream, _)) = accepted else { break; };
                    let mut request = [0u8; 1024];
                    let _ = stream.read(&mut request).await;
                    let body = r#"{"name":"openhuman","ok":true}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(), body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                }
            }
        }
    });

    let result = pick_listen_port_for_host("::1", port).await;
    let _ = shutdown_tx.send(());
    task.abort();
    let error = result.expect_err("an OpenHuman IPv6 listener should request takeover");
    assert!(
        matches!(error, PickListenPortError::WouldTakeOver { preferred, fingerprint } if preferred == port && fingerprint == "openhuman-core")
    );
}
