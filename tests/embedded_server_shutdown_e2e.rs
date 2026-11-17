//! Regression for #920: the embedded JSON-RPC server's `axum::serve` accept
//! loop must stop within the cancellation timeout when its `CancellationToken`
//! fires.
//!
//! This used to be a `#[ignore]`d unit test in `openhuman-rpc`
//! (`shims_tests.rs`, #1552). `host::serve_desktop` runs the full production
//! bootstrap, which spawns background tasks and writes process-global statics
//! (`scheduler_gate::STATE`, `SIGNED_OUT`, the LLM permit semaphore, the
//! `agent.run_turn` registry, ...) with no teardown, so inside the shared unit
//! test binary it raced with every sibling test. A root `tests/` target is its
//! own process, which is exactly the isolation the old ignore reason asked for.

use std::time::Duration;

use openhuman_rpc::host::{desktop_builder, serve_desktop, DesktopOptions};
use tokio_util::sync::CancellationToken;

async fn wait_until_port(port: u16, accepting: bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let connected = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok();
        if connected == accepting {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "port {port} never became accepting={accepting}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[test]
fn shutdown_token_stops_axum_listener_within_timeout() {
    // The core's boot is stack-hungry; give the runtime's threads room, as
    // `crates/openhuman-rpc/tests/host_desktop.rs` does.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .thread_stack_size(16 * 1024 * 1024)
                .build()
                .expect("tokio runtime")
                .block_on(shutdown_token_stops_listener())
        })
        .expect("test thread")
        .join()
        .expect("test thread should not panic");
}

async fn shutdown_token_stops_listener() {
    openhuman_core::platform::llm_gate::set_signed_out(false);

    let workspace = tempfile::tempdir().expect("workspace tempdir");
    // Pin the scheduler-gate policy so the bootstrap's `init_global` snapshot
    // cannot freeze the cached policy at Paused under CPU pressure.
    std::fs::write(
        workspace.path().join("config.toml"),
        "[scheduler_gate]\nmode = \"always_on\"\n",
    )
    .expect("seed scheduler_gate=always_on config.toml");
    // This target runs one test in its own process; nothing else reads these.
    std::env::set_var("OPENHUMAN_WORKSPACE", workspace.path());
    std::env::set_var("OPENHUMAN_DISABLE_CHANNEL_LISTENERS", "1");
    std::env::set_var("OPENHUMAN_CORE_TOKEN", "test-token-shutdown");
    // serve_desktop connects the TinyHumans layer; keep it off any real host.
    std::env::set_var("BACKEND_URL", "http://127.0.0.1:9");

    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("allocate test port");
    let port = probe.local_addr().expect("local addr").port();
    drop(probe);

    let shutdown_token = CancellationToken::new();
    let server_token = shutdown_token.clone();
    let options = DesktopOptions {
        host: Some("127.0.0.1".into()),
        port: Some(port),
        socketio: false,
        rpc_token: None,
    };
    let builder = desktop_builder(&options);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve_desktop(builder, server_token, ready_tx));

    // Full desktop bootstrap can exceed 30s on a cold CI runner while the
    // module search path and background services initialize. Match the host's
    // 60s core-ready budget; this test is about shutdown after readiness.
    let ready = match tokio::time::timeout(Duration::from_secs(60), ready_rx).await {
        Ok(Ok(ready)) => ready,
        Ok(Err(_)) => {
            let result = server.await.expect("embedded server task should not panic");
            panic!("embedded server ended before signaling readiness: {result:?}");
        }
        Err(_) => {
            server.abort();
            let _ = server.await;
            panic!("embedded server did not signal readiness within timeout");
        }
    };
    assert_eq!(
        ready.port, port,
        "embedded server selected an unexpected port"
    );

    wait_until_port(port, true).await;
    shutdown_token.cancel();

    let result = tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("embedded server task should stop within timeout")
        .expect("embedded server task should not panic");
    result.expect("embedded server should shut down cleanly");
    wait_until_port(port, false).await;
}
