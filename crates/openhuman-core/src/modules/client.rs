//! Shared module access for hosts, including before a core runtime is started.

use serde::{de::DeserializeOwned, Serialize};
use std::time::Duration;

use crate::config::Config;

use super::{failure, registry};

/// A sanitized terminal module outcome. Arguments and remote error text are absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModuleCallError {
    Unavailable,
    IncompatibleContract,
    TransportFailed,
    ModuleFault,
}

impl std::fmt::Display for ModuleCallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self {
            Self::Unavailable => "unavailable",
            Self::IncompatibleContract => "incompatible contract",
            Self::TransportFailed => "transport failed",
            Self::ModuleFault => "execution failed",
        };
        // Later product reporting recognises this as an already reported
        // module failure, rather than producing a second terminal event.
        write!(formatter, "MODULE_CALL_REPORTED: module {reason}")
    }
}

impl std::error::Error for ModuleCallError {}

/// Explicit configuration over the existing process-wide lazy module loader.
///
/// Constructing a client starts nothing. It needs neither a signed-in session
/// nor a core runtime. Set the bundled releases directory through the module
/// facade before the first call when artifacts ship with the host.
#[derive(Clone)]
pub struct ModuleClient {
    config: Config,
    timeout: Option<Duration>,
    #[cfg(all(test, feature = "modules"))]
    fixture: Option<tinybus::Proxy>,
}

impl ModuleClient {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            timeout: None,
            #[cfg(all(test, feature = "modules"))]
            fixture: None,
        }
    }

    #[cfg(all(test, feature = "modules"))]
    pub(crate) fn fixture(proxy: tinybus::Proxy) -> Self {
        Self {
            config: Config::default(),
            timeout: None,
            fixture: Some(proxy),
        }
    }

    #[cfg(test)]
    pub(crate) fn fixture_proxy(&self) -> Option<tinybus::Proxy> {
        self.fixture.clone()
    }

    /// Override the TinyBus deadline for every call made through this client.
    ///
    /// Use this for module operations whose contract has a longer bounded
    /// duration than TinyBus's default. It applies equally to confidential
    /// calls and test fixtures.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Execute a contract member, retaining its existing argument tuple arity.
    ///
    /// No implementation fallback is provided. The caller owns approvals,
    /// cancellation, and lifecycle cleanup; use [`Self::with_timeout`] for a
    /// longer module-specific deadline.
    pub async fn call<R: DeserializeOwned>(
        &self,
        module: &str,
        member: &str,
        args: impl Serialize,
    ) -> Result<R, ModuleCallError> {
        self.invoke(module, member, args, false).await
    }

    /// Execute a confidential member after TinyBus recipient attestation.
    ///
    /// Use this for credentials and other sensitive module inputs. A failure to
    /// attest the pinned artifact fails the call; ordinary delivery is never retried.
    pub async fn call_confidential<R: DeserializeOwned>(
        &self,
        module: &str,
        member: &str,
        args: impl Serialize,
    ) -> Result<R, ModuleCallError> {
        self.invoke(module, member, args, true).await
    }

    /// Call a member that returns a JSON-encoded string and deserialize that
    /// inner document. Malformed module output is reported using only the
    /// registered module identity and the closed `module_fault` reason.
    pub async fn call_json<R: DeserializeOwned>(
        &self,
        module: &str,
        member: &str,
        args: impl Serialize,
    ) -> Result<R, ModuleCallError> {
        let response: String = self.call(module, member, args).await?;
        decode_module_json(module, &response)
    }

    /// Confidential counterpart to [`Self::call_json`]. A decode failure is
    /// terminal; the call is never retried through ordinary delivery.
    pub async fn call_confidential_json<R: DeserializeOwned>(
        &self,
        module: &str,
        member: &str,
        args: impl Serialize,
    ) -> Result<R, ModuleCallError> {
        let response: String = self.call_confidential(module, member, args).await?;
        decode_module_json(module, &response)
    }

    /// Resolve a private proxy for an object returned by an owning module.
    ///
    /// Object paths are kept inside the owning host adapter; callers should
    /// wrap this in a domain-specific client rather than expose the path as
    /// product state.
    #[cfg(feature = "modules")]
    pub(crate) async fn object_client(
        &self,
        module: &str,
        object_path: &str,
    ) -> Result<ModuleObjectClient, ModuleCallError> {
        let Some(record) = registry::find(module) else {
            failure::report_unknown_module();
            return Err(ModuleCallError::Unavailable);
        };
        if !self.config.modules.enabled {
            failure::report(record, failure::Reason::Disabled);
            return Err(ModuleCallError::Unavailable);
        }
        #[cfg(test)]
        if let Some(fixture) = &self.fixture {
            let proxy = fixture
                .connection()
                .proxy(
                    fixture.destination().as_str(),
                    object_path,
                    fixture.interface().as_str(),
                )
                .map_err(|_| ModuleCallError::IncompatibleContract)?;
            let proxy = self
                .timeout
                .map_or(proxy.clone(), |timeout| proxy.with_timeout(timeout));
            return Ok(ModuleObjectClient {
                record,
                proxy,
                timeout: self.timeout,
            });
        }
        super::ops::ensure_loaded(&self.config, module)
            .await
            .map_err(|_| ModuleCallError::Unavailable)?;
        let runtime = super::host::runtime().await.map_err(|_| {
            failure::report(record, failure::Reason::TransportFailed);
            ModuleCallError::TransportFailed
        })?;
        let proxy = runtime.proxy(record.bus_name, object_path).map_err(|_| {
            failure::report(record, failure::Reason::IncompatibleContract);
            ModuleCallError::IncompatibleContract
        })?;
        let proxy = self
            .timeout
            .map_or(proxy.clone(), |timeout| proxy.with_timeout(timeout));
        Ok(ModuleObjectClient {
            record,
            proxy,
            timeout: self.timeout,
        })
    }

    /// Report a structurally invalid reply from a known module invocation.
    pub(crate) fn report_malformed_reply(module: &str) {
        if let Some(record) = registry::find(module) {
            failure::report_invocation(record, failure::Reason::ModuleFault);
        }
    }

    /// Report a registered module that advertises an older wire contract.
    pub(crate) fn report_incompatible_contract(module: &str) {
        if let Some(record) = registry::find(module) {
            failure::report(record, failure::Reason::IncompatibleContract);
        }
    }

    /// Report a known module that does not provide the requested capability.
    pub(crate) fn report_unavailable(module: &str) {
        if let Some(record) = registry::find(module) {
            failure::report(record, failure::Reason::ModuleUnavailable);
        }
    }

    async fn invoke<R: DeserializeOwned>(
        &self,
        module: &str,
        member: &str,
        args: impl Serialize,
        confidential: bool,
    ) -> Result<R, ModuleCallError> {
        let Some(record) = registry::find(module) else {
            failure::report_unknown_module();
            return Err(ModuleCallError::Unavailable);
        };
        if !self.config.modules.enabled {
            failure::report(record, failure::Reason::Disabled);
            return Err(ModuleCallError::Unavailable);
        }
        #[cfg(all(test, feature = "modules"))]
        if let Some(proxy) = self.fixture.as_ref() {
            let proxy = self.timeout.map_or_else(
                || proxy.clone(),
                |timeout| proxy.clone().with_timeout(timeout),
            );
            return invoke_proxy(record, &proxy, member, args, confidential).await;
        }
        #[cfg(not(feature = "modules"))]
        {
            let _ = (member, args, confidential);
            failure::report(record, failure::Reason::LoaderDisabled);
            Err(ModuleCallError::Unavailable)
        }
        #[cfg(feature = "modules")]
        {
            super::ops::ensure_loaded(&self.config, module)
                .await
                .map_err(|_| ModuleCallError::Unavailable)?;
            let runtime = super::host::runtime().await.map_err(|_| {
                failure::report(record, failure::Reason::TransportFailed);
                ModuleCallError::TransportFailed
            })?;
            let proxy = runtime
                .proxy(record.bus_name, record.object_path)
                .map_err(|_| {
                    failure::report(record, failure::Reason::IncompatibleContract);
                    ModuleCallError::IncompatibleContract
                })?;
            let proxy = self
                .timeout
                .map_or(proxy.clone(), |timeout| proxy.with_timeout(timeout));
            invoke_proxy(record, &proxy, member, args, confidential).await
        }
    }
}

/// A validated proxy for a module-owned object path.
///
/// This stays crate-private so bus object identities do not become product
/// state or leak into logs and controller responses.
#[cfg(feature = "modules")]
#[derive(Clone)]
pub(crate) struct ModuleObjectClient {
    record: &'static super::types::ModuleRecord,
    proxy: tinybus::Proxy,
    timeout: Option<Duration>,
}

#[cfg(feature = "modules")]
impl ModuleObjectClient {
    /// Capture the current unique peer before starting an owned lifecycle.
    /// Later alias replacement cannot redirect operations on this client.
    pub(crate) async fn pin_owner(&self) -> Result<Self, ModuleCallError> {
        let owner = self
            .proxy
            .connection()
            .name_owner(self.proxy.destination())
            .await
            .map_err(|error| report_proxy_failure(self.record, &error))?
            .ok_or(ModuleCallError::Unavailable)?;
        let proxy = self
            .proxy
            .connection()
            .proxy(
                owner.as_str(),
                self.proxy.path().as_str(),
                self.proxy.interface().as_str(),
            )
            .map_err(|error| report_proxy_failure(self.record, &error))?;
        let proxy = self
            .timeout
            .map_or(proxy.clone(), |timeout| proxy.with_timeout(timeout));
        Ok(Self {
            record: self.record,
            proxy,
            timeout: self.timeout,
        })
    }

    /// Derive a private child on the already captured module peer.
    pub(crate) fn object_at(&self, path: &str) -> Result<Self, ModuleCallError> {
        let proxy = self
            .proxy
            .connection()
            .proxy(
                self.proxy.destination().as_str(),
                path,
                self.proxy.interface().as_str(),
            )
            .map_err(|error| report_proxy_failure(self.record, &error))?;
        let proxy = self
            .timeout
            .map_or(proxy.clone(), |timeout| proxy.with_timeout(timeout));
        Ok(Self {
            record: self.record,
            proxy,
            timeout: self.timeout,
        })
    }

    pub(crate) async fn call<R: DeserializeOwned>(
        &self,
        member: &str,
        args: impl Serialize,
    ) -> Result<R, ModuleCallError> {
        invoke_proxy(self.record, &self.proxy, member, args, false).await
    }

    pub(crate) async fn call_confidential<R: DeserializeOwned>(
        &self,
        member: &str,
        args: impl Serialize,
    ) -> Result<R, ModuleCallError> {
        invoke_proxy(self.record, &self.proxy, member, args, true).await
    }
}

fn decode_module_json<R: DeserializeOwned>(
    module: &str,
    response: &str,
) -> Result<R, ModuleCallError> {
    serde_json::from_str(response).map_err(|_| {
        ModuleClient::report_malformed_reply(module);
        ModuleCallError::ModuleFault
    })
}

#[cfg(feature = "modules")]
async fn invoke_proxy<R: DeserializeOwned>(
    record: &'static super::types::ModuleRecord,
    proxy: &tinybus::Proxy,
    member: &str,
    args: impl Serialize,
    confidential: bool,
) -> Result<R, ModuleCallError> {
    let result = if confidential {
        proxy.call_confidential(member, args).await
    } else {
        proxy.call(member, args).await
    };
    result.map_err(|error| report_proxy_failure(record, &error))
}

/// Report a bus failure without exposing its payload to telemetry. Product
/// adapters may separately decode a structured provider refusal for display.
#[cfg(feature = "modules")]
pub(super) fn report_proxy_failure(
    record: &'static super::types::ModuleRecord,
    error: &tinybus::Error,
) -> ModuleCallError {
    let (outcome, reason) = classify(error);
    if reason == failure::Reason::ModuleUnavailable {
        failure::report(record, reason);
    } else {
        failure::report_invocation(record, reason);
    }
    outcome
}

#[cfg(feature = "modules")]
fn classify(error: &tinybus::Error) -> (ModuleCallError, failure::Reason) {
    use tinybus::Error;
    match error {
        Error::ModuleUnavailable { .. } => (
            ModuleCallError::Unavailable,
            failure::Reason::ModuleUnavailable,
        ),
        Error::IncompatibleVersion { .. }
        | Error::UnknownMethod { .. }
        | Error::UnknownInterface { .. }
        | Error::UnknownObject { .. }
        | Error::BadArguments { .. } => (
            ModuleCallError::IncompatibleContract,
            failure::Reason::IncompatibleContract,
        ),
        Error::MethodFailed { name, .. }
            if name == "ai.tinyhumans.tinybus.Error.ModuleUnavailable" =>
        {
            (
                ModuleCallError::Unavailable,
                failure::Reason::ModuleUnavailable,
            )
        }
        Error::MethodFailed { name, .. }
            if matches!(
                name.as_str(),
                Error::UNKNOWN_METHOD
                    | "ai.tinyhumans.tinybus.Error.IncompatibleVersion"
                    | "ai.tinyhumans.tinybus.Error.UnknownInterface"
                    | "ai.tinyhumans.tinybus.Error.UnknownObject"
                    | "ai.tinyhumans.tinybus.Error.BadArguments"
            ) =>
        {
            (
                ModuleCallError::IncompatibleContract,
                failure::Reason::IncompatibleContract,
            )
        }
        Error::Json(_) => (
            ModuleCallError::IncompatibleContract,
            failure::Reason::IncompatibleContract,
        ),
        Error::MethodFailed { name, .. }
            if matches!(
                name.as_str(),
                Error::NOT_ATTESTED | Error::CONFIDENTIALITY_REQUIRED
            ) =>
        {
            (
                ModuleCallError::TransportFailed,
                failure::Reason::TransportFailed,
            )
        }
        Error::MethodFailed { .. } => (ModuleCallError::ModuleFault, failure::Reason::ModuleFault),
        _ => (
            ModuleCallError::TransportFailed,
            failure::Reason::TransportFailed,
        ),
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
