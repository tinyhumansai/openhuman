//! [`RuntimeBuilder`]: an [`openhuman_embed::RuntimeBuilder`] that comes up
//! connected to the hosted TinyHumans backend.
//!
//! This is the configuration path for a TinyHumans-connected core. On
//! [`build`](RuntimeBuilder::build) (or [`connect`](RuntimeBuilder::connect))
//! it sets the product identity and hands the embed builder everything the
//! connection contributes, through embed's own seam options:
//!
//! | Piece | Embed option |
//! |---|---|
//! | the SDK transport ([`SdkBackendTransport`](crate::SdkBackendTransport)) | `backend_transport` (also the process global) |
//! | the hosted RPC proxies ([`hosted::extension`](crate::hosted::extension)) | `controller_extension` |
//! | the Jev `tool_search` ranker (`jev` feature) | `tool_ranker` |
//!
//! Every other method forwards to the embed builder unchanged. Start from a
//! host preset ([`library`](RuntimeBuilder::library),
//! [`desktop`](RuntimeBuilder::desktop), [`cli`](RuntimeBuilder::cli),
//! [`tui`](RuntimeBuilder::tui)) or [`from_embed`](RuntimeBuilder::from_embed);
//! [`into_embed`](RuntimeBuilder::into_embed) drops back to the plain,
//! unconnected builder. [`install`](crate::install) is the compatibility
//! entry for hosts that still boot the core themselves; both resolve the
//! connection through the same helper.

use std::path::PathBuf;
use std::sync::Arc;

use openhuman_embed::seams::{
    ControllerExtension, PostTurnHook, SecurityPolicy, ServerLauncher, ToolHook,
};
use tinytools::ToolRanker;

use openhuman_embed::{
    Access, ApiKey, ConfigSource, DomainSet, HostKind, Provider, Runtime, RuntimeConfig,
    ServiceSet, Session, SessionStoreProvider, TokenSource, ToolGroups, Workspace,
};

use crate::install::{wiring, InstallError, InstallOptions};

/// Build error: either the backend transport or the embed runtime failed.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// The TinyHumans transport could not be installed.
    #[error(transparent)]
    Install(#[from] InstallError),
    /// The embed runtime failed to build.
    #[error(transparent)]
    Embed(#[from] openhuman_embed::RuntimeError),
}

/// See the module docs.
#[derive(Default)]
pub struct RuntimeBuilder {
    inner: openhuman_embed::RuntimeBuilder,
    install: InstallOptions,
}

/// Forward a by-value embed builder method unchanged.
macro_rules! forward {
    ($(#[$doc:meta])* $name:ident($($arg:ident: $ty:ty),*)) => {
        $(#[$doc])*
        pub fn $name(mut self, $($arg: $ty),*) -> Self {
            self.inner = self.inner.$name($($arg),*);
            self
        }
    };
}

impl RuntimeBuilder {
    /// The embed defaults plus a TinyHumans backend connection on build.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start from an already-configured embed builder.
    pub fn from_embed(inner: openhuman_embed::RuntimeBuilder) -> Self {
        Self {
            inner,
            install: InstallOptions::default(),
        }
    }

    /// The library preset, connected; see [`openhuman_embed::RuntimeBuilder::library`].
    pub fn library() -> Self {
        Self::from_embed(openhuman_embed::RuntimeBuilder::library())
    }

    /// The desktop shell preset, connected; see [`openhuman_embed::RuntimeBuilder::desktop`].
    pub fn desktop() -> Self {
        Self::from_embed(openhuman_embed::RuntimeBuilder::desktop())
    }

    /// The CLI server preset, connected; see [`openhuman_embed::RuntimeBuilder::cli`].
    pub fn cli() -> Self {
        Self::from_embed(openhuman_embed::RuntimeBuilder::cli())
    }

    /// The terminal UI preset, connected; see [`openhuman_embed::RuntimeBuilder::tui`].
    pub fn tui() -> Self {
        Self::from_embed(openhuman_embed::RuntimeBuilder::tui())
    }

    /// The `x-sdk-name` this runtime reports to the backend. Set process-wide
    /// on build, before the transport captures its attribution headers.
    pub fn product_identity(mut self, identity: crate::backend::ProductIdentity) -> Self {
        self.install.product_identity = Some(identity);
        self
    }

    /// Whether the hosted RPC proxies (`billing`, `team`, …) are registered
    /// (default `true`; the runtime's `DomainSet` still gates them).
    pub fn hosted_controllers(mut self, enabled: bool) -> Self {
        self.install.hosted_controllers = enabled;
        self
    }

    /// Whether the Jev `tool_search` ranker is installed (default `true`;
    /// needs the `jev` feature). Off, the harness ranks with BM25 alone
    /// unless [`tool_ranker`](Self::tool_ranker) supplies another. A host
    /// ranker, however it was set, always wins over the Jev one.
    pub fn jev_ranker(mut self, enabled: bool) -> Self {
        self.install.tool_ranker = enabled;
        self
    }

    /// A `tool_search` ranker of the host's own, in place of the Jev ranker.
    /// See [`openhuman_embed::RuntimeBuilder::tool_ranker`].
    pub fn tool_ranker(mut self, ranker: Arc<dyn ToolRanker>) -> Self {
        self.install.tool_ranker = false;
        self.inner = self.inner.tool_ranker(ranker);
        self
    }

    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::workspace`].
        workspace(workspace: Workspace)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::config_source`].
        config_source(source: ConfigSource)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::provider`].
        provider(provider: Provider)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::access`].
        access(access: Access)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::services`].
        services(services: ServiceSet)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::defer_services`].
        defer_services()
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::domains`].
        domains(domains: DomainSet)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::tool_groups`].
        tool_groups(tool_groups: ToolGroups)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::host_kind`].
        host_kind(host_kind: HostKind)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::token`].
        token(token: TokenSource)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::listen_port`].
        listen_port(port: u16)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::session`].
        session(session: Session)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::config`].
        config(config: RuntimeConfig)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::session_store`].
        session_store(provider: Arc<dyn SessionStoreProvider>)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::memory_engine`].
        memory_engine(engine: Arc<dyn openhuman_embed::memory::api::MemoryEngine>)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::controller_extension`].
        /// The hosted proxies are added on build in addition to these.
        controller_extension(extension: ControllerExtension)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::post_turn_hook`].
        post_turn_hook(hook: Arc<dyn PostTurnHook>)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::tool_hook`].
        tool_hook(hook: Arc<dyn ToolHook>)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::server_launcher`].
        server_launcher(launcher: ServerLauncher)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::storage`].
        storage(source: openhuman_embed::seams::StorageSource)
    );
    forward!(
        /// See [`openhuman_embed::RuntimeBuilder::live_policy`].
        live_policy(policy: Arc<SecurityPolicy>)
    );

    /// See [`openhuman_embed::RuntimeBuilder::api_key`].
    pub fn api_key(mut self, key: impl Into<ApiKey>) -> Self {
        self.inner = self.inner.api_key(key);
        self
    }

    /// See [`openhuman_embed::RuntimeBuilder::backend_url`].
    pub fn backend_url(mut self, url: impl Into<String>) -> Self {
        self.inner = self.inner.backend_url(url);
        self
    }

    /// See [`openhuman_embed::RuntimeBuilder::workspace_dir`].
    pub fn workspace_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.inner = self.inner.workspace_dir(dir);
        self
    }

    /// See [`openhuman_embed::RuntimeBuilder::action_dir`].
    pub fn action_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.inner = self.inner.action_dir(dir);
        self
    }

    /// See [`openhuman_embed::RuntimeBuilder::listen`].
    pub fn listen(mut self, host: impl Into<String>, port: u16) -> Self {
        self.inner = self.inner.listen(host, port);
        self
    }

    /// See [`openhuman_embed::RuntimeBuilder::listen_host`].
    pub fn listen_host(mut self, host: impl Into<String>) -> Self {
        self.inner = self.inner.listen_host(host);
        self
    }

    /// The plain embed builder, with everything set so far but no backend
    /// connection.
    pub fn into_embed(self) -> openhuman_embed::RuntimeBuilder {
        self.inner
    }

    /// The embed builder with the TinyHumans connection applied: product
    /// identity set, the SDK transport installed as the process global and
    /// bound with `backend_transport`, the hosted proxies added as a
    /// controller extension and the Jev ranker as the tool ranker. For entry
    /// points that take an embed builder rather than this one.
    pub fn connect(self) -> Result<openhuman_embed::RuntimeBuilder, InstallError> {
        let wiring = wiring(&self.install)?;
        log::debug!("[tinyhumans] connecting embed builder: {wiring:?}");
        let transport: Arc<dyn openhuman_embed::BackendTransport> = wiring.transport;
        let mut inner = self.inner.backend_transport(transport);
        if let Some(extension) = wiring.controllers {
            inner = inner.controller_extension(extension);
        }
        if let Some(ranker) = wiring.ranker {
            // A ranker already on the embed builder (from `from_embed`, or
            // `tool_ranker` followed by `jev_ranker(true)`) is the host's
            // choice; the Jev ranker never replaces it.
            if inner.summary().tool_ranker.is_some() {
                log::debug!("[tinyhumans] host tool ranker kept; Jev ranker skipped");
            } else {
                inner = inner.tool_ranker(ranker);
            }
        }
        Ok(inner)
    }

    /// Connect, then boot the runtime.
    // Preserve the public error wrapper rather than box its `RuntimeError`.
    #[allow(clippy::result_large_err)]
    pub async fn build(self) -> Result<Runtime, RuntimeError> {
        let inner = self.connect()?;
        log::debug!("[tinyhumans] building embed runtime with backend transport");
        Ok(inner.build().await?)
    }

    /// Connect, then run the core's command-line dispatcher on `args`; see
    /// [`openhuman_embed::RuntimeBuilder::run_from_args`].
    pub fn run_from_args(self, args: &[String]) -> anyhow::Result<()> {
        self.connect()?.run_from_args(args)
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
