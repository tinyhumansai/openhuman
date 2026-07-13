//! Embed the OpenHuman core as a library — no HTTP, no background services.
//!
//! Demonstrates the pluggable-core API: build a fully-initialized core with
//! [`ServiceSet::none`] (no ports bound, no cron/channels/heartbeat) AND
//! [`DomainSet::harness`] (only the agent + memory + threads + config + security
//! domain families are live — the gate families flows/skills/mcp/meet/channels/
//! web3/voice/media and the catch-all `platform` are off, so their controllers
//! are unknown-method, their agent tools absent, and their stores/subscribers
//! never initialize). Dispatch RPC methods in-process through
//! [`CoreRuntime::invoke`] — the exact same path the HTTP `/rpc` handler and the
//! CLI use.
//!
//! Run with:
//!
//! ```bash
//! GGML_NATIVE=OFF cargo run --example embed_headless
//! ```
//!
//! To instead expose the core over HTTP for a single-core cloud deployment,
//! swap `ServiceSet::none()` for `ServiceSet::headless_api()`, keep a
//! `CancellationToken`, and call `runtime.serve(None, Some(token)).await` — it
//! binds `127.0.0.1:7788` (override with `.host(..)` / `.port(..)` on the
//! builder, or `OPENHUMAN_CORE_HOST` / `OPENHUMAN_CORE_PORT`) and serves until
//! the token is cancelled. Widen the runtime surface by swapping
//! `DomainSet::harness()` for `DomainSet::full()`.

use openhuman_core::{CoreBuilder, DomainSet, HostKind, ServiceSet};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Library embedders own logging; a simple env_logger keeps the example
    // self-contained. (`RUST_LOG=info cargo run --example embed_headless`)
    let _ = env_logger::builder().is_test(false).try_init();

    // Initialize the core against the local workspace. `HostKind::Cli` selects
    // the standalone (non-desktop) bootstrap path; `DomainSet::harness()` builds
    // the embeddable agent core; `ServiceSet::none()` means no transport and no
    // background services are started.
    let runtime = CoreBuilder::new(HostKind::Cli)
        .domains(DomainSet::harness())
        .services(ServiceSet::none())
        .build()
        .await?;

    // Dispatch a couple of RPC methods in-process — no network involved.
    // `core.version` and `openhuman.ping` (a legacy alias for the built-in
    // `core.ping`) are always available regardless of the DomainSet — they are
    // transport built-ins, not domain controllers — so they succeed even under
    // `harness()`.
    let version = runtime
        .invoke("core.version", serde_json::json!({}))
        .await
        .map_err(|e| anyhow::anyhow!("core.version failed: {e}"))?;
    println!("core.version -> {version}");

    let ping = runtime
        .invoke("openhuman.ping", serde_json::json!({}))
        .await
        .map_err(|e| anyhow::anyhow!("openhuman.ping failed: {e}"))?;
    println!("openhuman.ping -> {ping}");

    Ok(())
}
