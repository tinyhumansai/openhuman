//! Standalone server entry points, and the build-and-serve step the shared
//! host boot ([`crate::host`]) runs.
//!
//! `run_server` and `run_server_headless` are thin shims over the
//! [`RuntimeBuilder::cli`] preset: each applies the caller's services and
//! listener, then builds the runtime and [`serve`](super::serve::serve)s it.
//! `run_server_saas` is separate: it loads a `SaasConfig` and boots through
//! `core::runtime::saas::build` (a SaaS `CoreBuilder`, not the preset). The
//! preset carries every
//! domain family, the standalone `HostKind` (CLI or Docker, via
//! `HostKind::detect_standalone`), the `OPENHUMAN_E2E` tool-group switch, and
//! no supplied config (the core discovers the operator's install).
//!
//! None of these connect the TinyHumans backend: [`crate::host`] is the entry
//! that does (`desktop_builder` / `serve_desktop` for the embedded desktop
//! core, `cli` for `openhuman-core`). A caller of a bare `run_server*` installs
//! the transport itself (`openhuman_tinyhumans::install`).

use std::sync::Arc;

use openhuman_tinyhumans::embed::{RuntimeBuilder, ServiceSet, TokenSource};
use tokio_util::sync::CancellationToken;

use super::serve::EmbeddedReadySignal;

/// Resolves the port for the core server from environment variables or defaults.
pub(crate) fn core_port() -> u16 {
    std::env::var("OPENHUMAN_CORE_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(7788)
}

/// Resolves the bind address host for the core server from environment variables or defaults.
pub(crate) fn core_host() -> String {
    std::env::var("OPENHUMAN_CORE_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

/// Runs the HTTP/JSON-RPC server.
///
/// This function binds to the specified host and port, initializes the router,
/// bootstraps long-lived runtime infrastructure, and starts serving requests.
pub async fn run_server(
    host: Option<&str>,
    port: Option<u16>,
    socketio_enabled: bool,
) -> anyhow::Result<()> {
    run_server_inner(host, port, socketio_enabled).await
}

/// Runs the request/response-only HTTP API without detached background jobs.
pub async fn run_server_headless(host: Option<&str>, port: Option<u16>) -> anyhow::Result<()> {
    let services = ServiceSet::headless_api();
    run_server_with_services(host, port, services).await
}

/// Runs a SaaS core: many users behind a trusted gateway, booted from the
/// operator's config file and refused unless its boot guard passes.
///
/// The session store is installed before boot. With no storage URL every
/// profile keeps its sessions, transcripts and turn states in the default
/// database of its own workspace (the store resolves the workspace of the
/// context each call runs under). With one (`OPENHUMAN_STORAGE_URL`, else the operator's
/// `storage_url`) the backend is opened and installed — the profile registry
/// and leases live there too — and its session store never recovers on open:
/// recovery follows the profile lease instead.
///
/// On shutdown the leases of profiles nothing is using are released, so
/// another node can take them at once.
pub async fn run_server_saas(
    host: Option<&str>,
    port: Option<u16>,
    saas_config: &std::path::Path,
) -> anyhow::Result<()> {
    let config = crate::core_host::core::runtime::SaasConfig::load(saas_config)?;
    let storage_url = config.resolved_storage_url();
    log::info!(
        "[rpc:saas] session store: {}",
        if storage_url.is_some() {
            "storage backend"
        } else {
            "one database per profile workspace"
        }
    );
    crate::session_store::install_for_saas(storage_url).await?;
    let runtime =
        crate::core_host::core::runtime::saas::build(config, host.map(str::to_owned), port).await?;
    let served = super::serve::serve(&runtime, None, None).await;
    if let Some(profiles) = crate::core_host::profiles::host::host() {
        profiles.release_idle_on_shutdown().await;
    }
    served
}

/// Internal server entrypoint.
async fn run_server_inner(
    host: Option<&str>,
    port: Option<u16>,
    socketio_enabled: bool,
) -> anyhow::Result<()> {
    let mut services = ServiceSet::desktop();
    services.socketio = socketio_enabled;
    run_server_with_services(host, port, services).await
}

async fn run_server_with_services(
    host: Option<&str>,
    port: Option<u16>,
    services: ServiceSet,
) -> anyhow::Result<()> {
    let builder = server_builder(RuntimeBuilder::cli(), services, host, port, None);
    build_and_serve(builder, None, None).await
}

/// Apply a server's services, bearer and listener to a host preset.
///
/// `rpc_token` is the in-memory bearer handoff ([`TokenSource::Fixed`]);
/// `None` keeps the preset's env-or-file token. An unset host or port is left
/// to [`serve`](super::serve::serve), which falls back to
/// `OPENHUMAN_CORE_HOST` / `OPENHUMAN_CORE_PORT` and then the defaults. The
/// `OPENHUMAN_E2E` tool-group switch lives in the host presets, so a builder a
/// host narrowed itself keeps its tool groups.
pub(crate) fn server_builder(
    preset: RuntimeBuilder,
    services: ServiceSet,
    host: Option<&str>,
    port: Option<u16>,
    rpc_token: Option<Arc<String>>,
) -> RuntimeBuilder {
    let mut builder = preset.services(services);
    if let Some(token) = rpc_token {
        builder = builder.token(TokenSource::Fixed(token));
    }
    if let Some(host) = host {
        builder = builder.listen_host(host);
    }
    if let Some(port) = port {
        builder = builder.listen_port(port);
    }
    builder
}

/// Build `builder`'s runtime and serve it until shutdown.
///
/// The on-disk session store is installed for the life of the process, as the
/// shims always did, rather than through `RuntimeBuilder::session_store`
/// (which restores the previous provider when the runtime drops). The desktop
/// restarts its in-process server in place; a detached turn still writing
/// across that gap must keep landing in the classic layout, not in the
/// core's no-provider fallback.
pub(crate) async fn build_and_serve(
    builder: RuntimeBuilder,
    ready_tx: Option<tokio::sync::oneshot::Sender<EmbeddedReadySignal>>,
    shutdown_token: Option<CancellationToken>,
) -> anyhow::Result<()> {
    let summary = builder.summary();
    log::debug!(
        "[rpc:server] building runtime host_kind={:?} services={:?} fixed_token={} \
         listen_host={:?} listen_port={:?} embedded_ready={}",
        summary.host_kind,
        summary.services,
        summary.fixed_token,
        summary.listen_host,
        summary.listen_port,
        ready_tx.is_some()
    );

    // The desktop app and the CLI keep conversations in the storage backend
    // the host's configuration names (by default `sqlite:<workspace>/storage`);
    // the core itself carries no storage. Installed before boot so its
    // recovery sweep runs. A builder
    // that brings its own session store skips this: its provider is installed
    // by `build()`, and an unrelated host storage URL must not block it.
    if summary.has_session_store {
        log::debug!("[rpc:server] builder carries a session store; host store setup skipped");
    } else {
        crate::session_store::install_for_host().await?;
    }
    let runtime = builder.build().await.map_err(|error| {
        log::warn!("[rpc:server] runtime build failed: {error}");
        anyhow::Error::new(error)
    })?;
    let served = super::serve::serve(runtime.core_runtime(), ready_tx, shutdown_token).await;
    // Dropping the runtime releases the process's embed runtime slot, so the
    // desktop can restart its server in place.
    drop(runtime);
    log::debug!("[rpc:server] runtime released ok={}", served.is_ok());
    served
}

#[cfg(test)]
#[path = "shims_tests.rs"]
mod tests;
