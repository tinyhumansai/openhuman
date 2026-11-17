//! The server behind `openhuman-core run` / `serve`.

use std::future::Future;
use std::pin::Pin;

use openhuman_tinyhumans::embed::{RuntimeBuilder, ServiceSet};

use crate::core_host::core::server_launcher::ServeRequest;

/// The [`ServerLauncher`](openhuman_tinyhumans::embed::seams::ServerLauncher)
/// behind `run` / `serve`: the standalone server shims.
///
/// Boots from the builder the host handed the CLI
/// ([`RuntimeBuilder::run_from_args`]), or the `cli` preset when there is
/// none; see [`cli_builder`] for how the explicit flags are applied on top.
pub(crate) fn launch(
    mut request: ServeRequest,
) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
    Box::pin(async move {
        log::debug!(
            "[rpc:cli] starting server host={:?} port={:?} socketio={} headless_api={} mode={} \
             host_builder={}",
            request.host,
            request.port,
            request.socketio_enabled,
            request.headless_api,
            request.mode,
            request.host_boot.is_some()
        );
        if request.mode == crate::core_host::core::runtime::Mode::Saas {
            // A SaaS core boots from the operator's config file; the host's
            // builder has no say in it.
            let config = request
                .saas_config
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("--mode saas needs --saas-config <file>"))?;
            super::run_server_saas(request.host.as_deref(), request.port, config).await
        } else {
            let base = request
                .host_boot
                .take()
                .and_then(|boot| RuntimeBuilder::from_host_boot(&boot));
            let builder = flagged_builder(base.unwrap_or_else(RuntimeBuilder::cli), &request);
            super::shims::build_and_serve(builder, None, None).await
        }
    })
}

/// Apply the operator's explicit `run` / `serve` flags on top of `base`.
///
/// `--headless-api` replaces the services with the headless set;
/// `--jsonrpc-only` clears Socket.IO on whatever services `base` carries (the
/// `cli` preset's, when the host set none); `--host` / `--port` replace the
/// builder's listener. A flag the operator did not pass leaves `base` alone.
pub(crate) fn flagged_builder(base: RuntimeBuilder, request: &ServeRequest) -> RuntimeBuilder {
    let services = if request.headless_api {
        ServiceSet::headless_api()
    } else {
        let mut services = base.summary().services.unwrap_or_else(ServiceSet::desktop);
        if !request.socketio_enabled {
            services.socketio = false;
        }
        services
    };
    super::shims::server_builder(base, services, request.host.as_deref(), request.port, None)
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
