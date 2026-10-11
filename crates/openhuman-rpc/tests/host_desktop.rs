//! `host::serve_desktop` booted in-process: ready signal, `GET /health`, then a
//! clean shutdown. Its own test binary because a runtime claims a process-wide
//! slot (the keyring, event bus and domain subscribers are process-scoped).
#![cfg(feature = "server")]

use std::sync::Arc;
use std::time::Duration;

use openhuman_rpc::embed::ServiceSet;
use openhuman_rpc::host::{desktop_builder, serve_desktop, DesktopOptions, EmbeddedReadySignal};
use tokio_util::sync::CancellationToken;

/// HTTP only: no cron, channels, update checker or other background work that
/// could reach beyond loopback.
fn http_only() -> ServiceSet {
    ServiceSet {
        rpc_http: true,
        socketio: false,
        cron: false,
        channels: false,
        login_gated: false,
        update_scheduler: false,
        memory_queue: false,
        skill_catalog_refresh: false,
        mcp_boot: false,
        integrations: false,
        memory_sync: false,
    }
}

fn free_loopback_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

#[test]
fn serve_desktop_signals_ready_serves_health_and_stops_on_shutdown() {
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    std::env::set_var("OPENHUMAN_WORKSPACE", workspace.path());
    // Isolate from the operator's environment: a storage URL would route the
    // session store to a real backend, and a backend URL to a real host.
    std::env::remove_var("OPENHUMAN_STORAGE_URL");
    std::env::set_var("BACKEND_URL", "http://127.0.0.1:9");
    std::env::set_var("OPENHUMAN_CORE_HOST", "127.0.0.1");

    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(16 * 1024 * 1024)
                .build()
                .expect("tokio runtime")
                .block_on(async {
                    let options = DesktopOptions {
                        host: Some("127.0.0.1".into()),
                        port: Some(free_loopback_port()),
                        socketio: false,
                        rpc_token: Some(Arc::new("host-desktop-test-bearer".into())),
                    };
                    let builder = desktop_builder(&options).services(http_only());
                    let shutdown = CancellationToken::new();
                    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
                    let server = tokio::spawn(serve_desktop(builder, shutdown.clone(), ready_tx));

                    let EmbeddedReadySignal { port, .. } =
                        tokio::time::timeout(Duration::from_secs(120), ready_rx)
                            .await
                            .expect("ready signal within the timeout")
                            .expect("ready signal sent");
                    // The listener may fall back to another port if the one we
                    // probed was taken in between; use what it reports.

                    let health = reqwest::Client::new()
                        .get(format!("http://127.0.0.1:{port}/health"))
                        .send()
                        .await
                        .expect("GET /health");
                    assert_eq!(health.status(), 200);

                    shutdown.cancel();
                    let finished = tokio::time::timeout(Duration::from_secs(60), server)
                        .await
                        .expect("server stops after shutdown")
                        .expect("server task joins");
                    finished.expect("server exits cleanly");
                });
        })
        .expect("test thread")
        .join()
        .expect("test thread should not panic");
}
