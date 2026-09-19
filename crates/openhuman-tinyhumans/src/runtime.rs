//! [`RuntimeBuilder`]: an [`openhuman_embed::RuntimeBuilder`] that comes up
//! connected to the hosted TinyHumans backend.
//!
//! Every method delegates to the embed builder unchanged; `build()` installs
//! the SDK transport, binds it to the runtime's context and boots. Use
//! [`RuntimeBuilder::into_embed`] to drop down to the plain builder.

use std::sync::Arc;

use openhuman_embed::{
    Access, ApiKey, DomainSet, HostKind, Provider, Runtime, RuntimeConfig, ServiceSet, Session,
    ToolGroups, Workspace,
};

use crate::install::{install, InstallError, InstallOptions};

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

    /// The `x-sdk-name` this runtime reports to the backend.
    pub fn product_identity(mut self, identity: openhuman_embed::ProductIdentity) -> Self {
        self.install.product_identity = Some(identity);
        self
    }

    /// Whether the hosted RPC proxies (`billing`, `team`, …) are registered
    /// (default `true`; the runtime's `DomainSet` still gates them).
    pub fn hosted_controllers(mut self, enabled: bool) -> Self {
        self.install.hosted_controllers = enabled;
        self
    }

    /// See [`openhuman_embed::RuntimeBuilder::workspace`].
    pub fn workspace(mut self, workspace: Workspace) -> Self {
        self.inner = self.inner.workspace(workspace);
        self
    }

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

    /// See [`openhuman_embed::RuntimeBuilder::provider`].
    pub fn provider(mut self, provider: Provider) -> Self {
        self.inner = self.inner.provider(provider);
        self
    }

    /// See [`openhuman_embed::RuntimeBuilder::access`].
    pub fn access(mut self, access: Access) -> Self {
        self.inner = self.inner.access(access);
        self
    }

    /// See [`openhuman_embed::RuntimeBuilder::services`].
    pub fn services(mut self, services: ServiceSet) -> Self {
        self.inner = self.inner.services(services);
        self
    }

    /// See [`openhuman_embed::RuntimeBuilder::domains`].
    pub fn domains(mut self, domains: DomainSet) -> Self {
        self.inner = self.inner.domains(domains);
        self
    }

    /// See [`openhuman_embed::RuntimeBuilder::tool_groups`].
    pub fn tool_groups(mut self, tool_groups: ToolGroups) -> Self {
        self.inner = self.inner.tool_groups(tool_groups);
        self
    }

    /// See [`openhuman_embed::RuntimeBuilder::host_kind`].
    pub fn host_kind(mut self, host_kind: HostKind) -> Self {
        self.inner = self.inner.host_kind(host_kind);
        self
    }

    /// See [`openhuman_embed::RuntimeBuilder::session`].
    pub fn session(mut self, session: Session) -> Self {
        self.inner = self.inner.session(session);
        self
    }

    /// See [`openhuman_embed::RuntimeBuilder::config`].
    pub fn config(mut self, config: RuntimeConfig) -> Self {
        self.inner = self.inner.config(config);
        self
    }

    /// The plain embed builder, with everything set so far but no backend
    /// connection.
    pub fn into_embed(self) -> openhuman_embed::RuntimeBuilder {
        self.inner
    }

    /// Install the TinyHumans transport, bind it and boot the runtime.
    pub async fn build(self) -> Result<Runtime, RuntimeError> {
        let transport = install(self.install)?;
        let transport: Arc<dyn openhuman_embed::BackendTransport> = transport;
        log::debug!("[tinyhumans] building embed runtime with backend transport");
        Ok(self.inner.backend_transport(transport).build().await?)
    }
}
