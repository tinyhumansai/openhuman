//! The CLI entry: [`RuntimeBuilder::run_from_args`] and [`run_from_args`].
//!
//! `openhuman_core::run_core_from_args_with` dispatches a whole command line —
//! `run`/`serve`, `call`, `agent`, `mcp`, the namespace commands. Only
//! `run`/`serve` boot a runtime, and they do it through the installed
//! [`server_launcher`](RuntimeBuilder::server_launcher). So the entry is two
//! steps:
//!
//! 1. the builder's process-global pieces (backend transport, memory engine,
//!    session store, controller extensions, launcher, hooks) are installed for
//!    the life of the process, which every subcommand needs;
//! 2. what is left of the builder (host kind, domains, services, token,
//!    listener, workspace, tool groups, ...) is handed to the core as a
//!    [`HostBoot`], which rides the `run`/`serve` [`ServeRequest`] to the
//!    launcher. The launcher boots from it and applies the operator's explicit
//!    flags on top (`--host`, `--port`, `--jsonrpc-only`, `--headless-api`);
//!    see [`ServeRequest`].
//!
//! [`ServeRequest`]: openhuman_core::core::server_launcher::ServeRequest

use std::sync::Arc;

use openhuman_core::backend::BackendTransport;
use openhuman_core::core::server_launcher::HostBoot;
use tinyagents_session::port::SessionStoreProvider;

use super::seams::{HostSeams, StorageSource};
use super::{RuntimeBuilder, RuntimeError};

/// The pieces of a builder that are installed process-wide before the CLI
/// dispatches, split from the rest, which becomes the [`HostBoot`].
pub(super) struct CliGlobals {
    pub(super) transport: Option<Arc<dyn BackendTransport>>,
    pub(super) memory_engine: Option<Arc<dyn tinymemory_api::MemoryEngine>>,
    pub(super) session_store: Option<Arc<dyn SessionStoreProvider>>,
    pub(super) seams: HostSeams,
}

impl RuntimeBuilder {
    /// Install this builder's process-global pieces, then run the core's
    /// command-line dispatcher on `args` (without the binary name).
    ///
    /// Installed for the rest of the process, never restored: the backend
    /// transport (as the process global; the builder keeps it too so the
    /// server's runtime binds it), the memory engine, the session store,
    /// controller extensions, the server launcher, the tool ranker and the
    /// embedder hooks.
    ///
    /// For `run` / `serve` the rest of the builder is what the server boots
    /// with. Precedence, highest first: the operator's explicit flags
    /// (`--host`, `--port`, `--jsonrpc-only`, `--headless-api`, `--mode`),
    /// then this builder, then the launcher's own preset. `--mode saas` boots
    /// the operator's SaaS config instead and takes nothing from the builder.
    /// Other subcommands run in-process and use only the installed globals.
    ///
    /// The CLI initializes the keyring from the operator's install before any
    /// subcommand runs, so this entry is for [`Workspace::Inherit`](crate::Workspace::Inherit)
    /// (the `cli` preset); another workspace logs a warning.
    ///
    /// A [`live_policy`](Self::live_policy) is ignored with a warning — each
    /// subcommand's boot installs its own.
    ///
    /// # Errors
    ///
    /// A refused controller extension, or whatever the dispatched command
    /// returns.
    pub fn run_from_args(self, args: &[String]) -> anyhow::Result<()> {
        let command = args.first().map(String::as_str).unwrap_or("<none>");
        log::debug!(
            "[embed][cli] run_from_args command={command} argc={}",
            args.len()
        );

        if !self.workspace.is_operator_owned() {
            // The CLI latches the keyring and credential store to the
            // operator's install before any launcher runs, so a builder
            // workspace elsewhere would not move them.
            log::warn!(
                "[embed][cli] run_from_args expects Workspace::Inherit; the keyring and \
                 credentials stay with the operator's install"
            );
        }
        let (mut builder, globals) = self.split_for_cli();
        let storage = globals
            .install()
            .map_err(|error| anyhow::Error::new(RuntimeError::Invalid(error)))?;
        // The server's runtime installs the same, already opened backend
        // rather than opening the URL a second time.
        if let Some(backend) = storage {
            builder.seams.storage = Some(StorageSource::Backend(backend));
        }

        let summary = builder.summary();
        log::debug!(
            "[embed][cli] handing builder to the core host_kind={:?} domains={:?} \
             services={:?} fixed_token={} listen_host={:?} listen_port={:?}",
            summary.host_kind,
            summary.domains,
            summary.services,
            summary.fixed_token,
            summary.listen_host,
            summary.listen_port,
        );
        openhuman_core::run_core_from_args_with(args, Some(HostBoot::new(builder)))
    }

    /// Separate the process-global pieces from the builder that boots the
    /// server. The transport, memory engine and session store stay on the
    /// builder as well: the server's runtime binds the transport, and
    /// `build()` validates a `Workspace::Stateless` workspace against the
    /// session store and installs the custom engine/store over the
    /// launcher's defaults.
    pub(super) fn split_for_cli(mut self) -> (RuntimeBuilder, CliGlobals) {
        if self.seams.live_policy.take().is_some() {
            log::warn!(
                "[embed][cli] live_policy is ignored by run_from_args; \
                 each subcommand installs the policy its config describes"
            );
        }
        // The storage seam stays on the builder as well: like the session
        // store, the server's runtime binds it.
        let storage = self.seams.storage.clone();
        let mut seams = std::mem::take(&mut self.seams);
        seams.storage = storage.clone();
        self.seams.storage = storage;
        let globals = CliGlobals {
            transport: self.backend_transport.clone(),
            memory_engine: self.memory_engine.clone(),
            session_store: self.session_store.clone(),
            seams,
        };
        (self, globals)
    }

    /// Take back the builder [`run_from_args`](Self::run_from_args) handed
    /// the core, for a [`ServerLauncher`](crate::seams::ServerLauncher) to
    /// boot from. `None` when the request carries none (the launcher then
    /// uses its own preset) or it was already taken.
    pub fn from_host_boot(boot: &HostBoot) -> Option<RuntimeBuilder> {
        boot.take::<RuntimeBuilder>()
    }
}

impl CliGlobals {
    /// Install every global; returns the storage backend, when one was
    /// configured, now opened.
    fn install(
        mut self,
    ) -> Result<Option<Arc<dyn openhuman_core::storage::StorageBackend>>, String> {
        if let Some(transport) = self.transport {
            openhuman_core::backend::install_backend_transport(transport);
            log::debug!("[embed][cli] backend transport installed (process global)");
        }
        if let Some(engine) = self.memory_engine {
            openhuman_core::memory::engine::install_host_engine(engine);
        }
        if let Some(provider) = self.session_store {
            let _previous = openhuman_core::agent::session_store::install(provider);
            log::debug!("[embed][cli] session store installed (process lifetime)");
        }
        self.seams.open_storage_blocking()?;
        let storage = match &self.seams.storage {
            Some(StorageSource::Backend(backend)) => Some(Arc::clone(backend)),
            _ => None,
        };
        let seams = self.seams.install()?;
        seams.persist();
        Ok(storage)
    }
}

/// Run the core's command-line dispatcher with the [`RuntimeBuilder::cli`]
/// preset — no backend transport, extensions or launcher. Hosts that need
/// those (the TinyHumans transport, the JSON-RPC server) configure a builder
/// and call [`RuntimeBuilder::run_from_args`] instead.
pub fn run_from_args(args: &[String]) -> anyhow::Result<()> {
    RuntimeBuilder::cli().run_from_args(args)
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
