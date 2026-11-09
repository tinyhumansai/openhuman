//! `host::desktop` booted in-process: ready signal, `GET /health`, then a
//! clean shutdown. Its own test binary because a runtime claims a process-wide
//! slot (the keyring, event bus and domain subscribers are process-scoped).
#![cfg(feature = "server")]

use std::sync::Arc;
use std::time::Duration;

use openhuman_rpc::embed::ServiceSet;
use openhuman_rpc::host::{desktop_builder, serve_desktop, DesktopOptions, EmbeddedReadySignal};
use tokio_util::sync::CancellationToken;

/// HTTP only: no cron, channels, update checker or other background work.
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
    let config_dir = workspace.path().join(".openhuman");
    std::fs::create_dir_all(&config_dir).expect("config directory");
    std::fs::write(
        config_dir.join("config.toml"),
        "[secrets]\nencrypt = false\n",
    )
    .expect("unencrypted test config");
    std::env::set_var("HOME", workspace.path());
    std::env::set_var("USERPROFILE", workspace.path());
    std::env::set_var("TOKIO_WORKER_THREADS", "2");
    std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "memory");
    std::env::set_var(
        "OPENHUMAN_KEYRING_MASTER_KEY",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    );
    std::env::set_var("OPENHUMAN_WORKSPACE", workspace.path());
    // Isolate from the operator's environment: a storage URL would route the
    // session store to a real backend, and a backend URL to a real host.
    std::env::remove_var("OPENHUMAN_STORAGE_URL");
    std::env::set_var("BACKEND_URL", "http://127.0.0.1:9");
    std::env::set_var("OPENHUMAN_CORE_HOST", "127.0.0.1");

    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .thread_stack_size(16 * 1024 * 1024)
                .build()
                .expect("tokio runtime");
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runtime.block_on(async {
                    let options = DesktopOptions {
                        host: Some("127.0.0.1".into()),
                        port: Some(free_loopback_port()),
                        socketio: false,
                        rpc_token: Some(Arc::new("host-desktop-test-bearer".into())),
                    };
                    let shutdown = CancellationToken::new();
                    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
                    let builder = desktop_builder(&options).services(http_only());
                    let mut server = tokio::spawn(serve_desktop(
                        builder,
                        shutdown.clone(),
                        ready_tx,
                    ));
                    let ready = match tokio::time::timeout(Duration::from_secs(120), ready_rx).await
                    {
                        Ok(Ok(ready)) => ready,
                        Ok(Err(error)) => {
                            let result = tokio::time::timeout(Duration::from_secs(5), &mut server)
                                .await;
                            panic!("desktop server closed ready channel ({error}); join: {result:?}");
                        }
                        Err(_) => panic!("ready signal within the timeout"),
                    };
                    let EmbeddedReadySignal { port, .. } = ready;
                    // The listener may fall back to another port if the one we
                    // probed was taken in between; use what it reports.

                    let client = reqwest::Client::builder()
                        .pool_max_idle_per_host(0)
                        .build()
                        .expect("HTTP client");
                    let health = tokio::select! {
                        result = tokio::time::timeout(
                            Duration::from_secs(10),
                            client
                                .get(format!("http://127.0.0.1:{port}/health"))
                                .header(reqwest::header::CONNECTION, "close")
                                .send(),
                        ) => result.expect("GET /health completes during startup preflight")
                            .expect("GET /health"),
                        result = &mut server => panic!("desktop server ended before health: {result:?}"),
                    };
                    assert_eq!(health.status(), 200);
                    // Consume the body so the pooled HTTP connection is
                    // complete before graceful shutdown waits for it.
                    let _body = health.bytes().await.expect("health response body");
                    drop(client);

                    shutdown.cancel();
                    let finished = tokio::time::timeout(Duration::from_secs(60), server)
                        .await
                        .expect("server stops after shutdown")
                        .expect("server task joins");
                    finished.expect("server exits cleanly");
                });
            }));
            runtime.shutdown_timeout(Duration::from_secs(1));
            if let Err(payload) = result {
                std::panic::resume_unwind(payload);
            }
        })
        .expect("test thread")
        .join()
        .expect("test thread should not panic");
}
