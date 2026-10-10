//! Shared module access for hosts, including before a core runtime is started.

use serde::{de::DeserializeOwned, Serialize};

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
}

impl ModuleClient {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Execute a contract member, retaining its existing argument tuple arity.
    ///
    /// No implementation fallback is provided. The caller owns approvals,
    /// cancellation, deadlines and lifecycle cleanup members.
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
            invoke_proxy(record, &proxy, member, args, confidential).await
        }
    }
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
    result.map_err(|error| {
        let (outcome, reason) = classify(&error);
        if reason == failure::Reason::ModuleUnavailable {
            // TinyBus emits this stable error for a module whose terminal
            // lifecycle state is cached for the rest of the process.
            failure::report(record, reason);
        } else {
            failure::report_invocation(record, reason);
        }
        outcome
    })
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
