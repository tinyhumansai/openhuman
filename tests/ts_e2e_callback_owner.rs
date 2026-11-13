//! Host callbacks on the module runtime's shared connection, called by a peer
//! that is not the module they exist for (TS-E2E-012). Run each test by exact
//! name in its own process after `source scripts/ci/ts-e2e-stage-modules.sh`.

#[allow(dead_code)]
#[path = "support/rogue_peer.rs"]
mod rogue_peer;
#[allow(dead_code)]
#[path = "support/ts_e2e_host.rs"]
mod ts_e2e_host;
#[allow(dead_code)]
#[path = "support/ts_e2e_modules.rs"]
mod ts_e2e_modules;

use rogue_peer::Delivery;
use tinyeventqueues_bus::names::host as inbound;
use tinyeventqueues_bus::{DeliverRequest, DeliverResponse, Lane, Turn};
use ts_e2e_host::{run_on_agent_stack, Host};

/// CB-36: TinyEventQueues' `Deliver` host callback answers only the
/// TinyEventQueues module. The caller here is the bus's controller; another
/// loaded module would be treated the same, since the member never reads its
/// sender (inbound/host.rs:33-53). Any reply, even a rejection, means the
/// forged turn reached the deliverer.
#[test]
#[ignore = "needs the staged module releases (scripts/ci/ts-e2e-stage-modules.sh)"]
fn cb_36_inbound_deliver_refuses_a_caller_that_is_not_the_module() {
    run_on_agent_stack("cb_36", || async {
        let scratch = tempfile::tempdir().expect("scratch");
        let host = Host::start(scratch.path(), |_| {}).await;
        assert!(
            rogue_peer::is_served(inbound::INTERFACE, inbound::OBJECT_PATH, inbound::INTERFACE)
                .await,
            "the inbound host callbacks must be served for this row to mean anything"
        );
        let forged = DeliverRequest::new(Turn::new(
            "ts-e2e-cb36-forged-turn",
            Lane::Digest,
            "ts-e2e-cb36-conversation",
            Vec::new(),
        ));
        let reply: tinybus::Result<DeliverResponse> = rogue_peer::call(
            inbound::INTERFACE,
            inbound::OBJECT_PATH,
            inbound::INTERFACE,
            inbound::methods::DELIVER,
            forged,
            Delivery::Plain,
        )
        .await;
        assert!(
            reply.is_err(),
            "a Deliver from a peer that is not TinyEventQueues was handled: {reply:?}"
        );
        host.stop().await;
    });
}

/// CB-39: the wallet host's `WalletBackend` (an authenticated TinyHumans
/// backend transport) answers only the TinyWallet module. The caller is the
/// bus's controller (X5); a forged request must never reach the backend.
#[test]
#[ignore = "needs the staged module releases (scripts/ci/ts-e2e-stage-modules.sh)"]
fn cb_39_wallet_backend_refuses_a_caller_that_is_not_the_module() {
    run_on_agent_stack("cb_39", || async {
        let backend = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock backend");
        let backend_url = format!("http://{}", backend.local_addr().expect("addr"));
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = hits.clone();
        // Counts wallet requests (a forwarded WalletBackend call is a GET under
        // /agent-integrations/crypto/) and answers every request with an empty
        // JSON 200, so a caller never waits on it (CF-1: a silent mock hung
        // auth_store_session). The host's own traffic after sign-in is not
        // counted.
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut socket, _)) = backend.accept().await {
                let counter = counter.clone();
                tokio::spawn(async move {
                    let mut request = [0u8; 8192];
                    let read = socket.read(&mut request).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&request[..read]);
                    let line = head.lines().next().unwrap_or_default().to_owned();
                    eprintln!("[cb_39] mock backend request: {line}");
                    if line.contains("/crypto/") {
                        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    let _ = socket
                        .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}")
                        .await;
                });
            }
        });
        let scratch = tempfile::tempdir().expect("scratch");
        let host = Host::start_with_backend(scratch.path(), &backend_url, |_| {}).await;
        Host::result(
            &host
                .rpc(
                    "openhuman.auth_store_session",
                    serde_json::json!({"token": "ts-e2e-jwt", "user_id": "ts-e2e-user"}),
                )
                .await,
        );
        const WALLET: &str = "ai.tinyhumans.tinywallet.Host";
        const PATH: &str = "/ai/tinyhumans/tinywallet/Host";
        assert!(
            rogue_peer::is_served(WALLET, PATH, WALLET).await,
            "the wallet host callbacks must be served for this row to mean anything"
        );
        let before = hits.load(std::sync::atomic::Ordering::SeqCst);
        let reply: tinybus::Result<serde_json::Value> = rogue_peer::call(
            WALLET,
            PATH,
            WALLET,
            "WalletBackend",
            serde_json::json!({"kind": "routes"}),
            Delivery::Plain,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let forwarded = hits.load(std::sync::atomic::Ordering::SeqCst) - before;
        // The wallet host's own error name means its handler ran (and tried
        // the backend); a refused caller gets the bus's error instead.
        let reached_handler = format!("{reply:?}").contains("tinywallet.Host.Error");
        assert!(
            forwarded == 0 && reply.is_err() && !reached_handler,
            "a WalletBackend call from a peer that is not TinyWallet was handled \
             ({forwarded} wallet backend requests): {reply:?}"
        );
        host.stop().await;
    });
}
