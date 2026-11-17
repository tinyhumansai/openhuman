//! Native permission operations through TinyComputer.

use serde::{de::DeserializeOwned, Serialize};
use tinycomputer_bus::{names::accessibility as methods, DesktopError, DesktopResponse};

use crate::config::Config;
use tinybus::module::{ModuleCallError, ModuleClient};

pub use tinycomputer_bus::accessibility::{PermissionKind, PermissionState, PermissionStatus};

const MODULE: &str = "tinycomputer";

/// A transport failure or the module's structured native refusal.
#[derive(Debug)]
pub enum AccessibilityError {
    Bus(ModuleCallError),
    Operation(DesktopError),
}

impl std::fmt::Display for AccessibilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bus(error) => error.fmt(f),
            Self::Operation(error) => f.write_str(&error.message),
        }
    }
}
impl std::error::Error for AccessibilityError {}

/// Explicit configuration over the process-wide lazy module loader.
#[derive(Clone)]
pub struct AccessibilityClient {
    client: ModuleClient,
    /// `[components] computer`. Off refuses every call before the module is
    /// touched; this client is compiled in unconditionally, so it carries the
    /// ceiling itself.
    computer_enabled: bool,
}

impl AccessibilityClient {
    pub fn new(config: &Config) -> Self {
        Self {
            client: crate::modules::client(config.clone()),
            computer_enabled: config.components.computer,
        }
    }

    #[cfg(all(test, feature = "modules"))]
    pub(crate) fn fixture(proxy: tinybus::Proxy) -> Self {
        Self {
            client: crate::modules::ops::fixture_client(proxy),
            computer_enabled: true,
        }
    }

    async fn call<T: DeserializeOwned>(
        &self,
        member: &str,
        args: impl Serialize,
    ) -> Result<T, AccessibilityError> {
        if !self.computer_enabled {
            log::debug!("[accessibility] {member} refused, component 'computer' disabled");
            return Err(AccessibilityError::Operation(DesktopError::new(
                crate::core::runtime::components::COMPONENT_DISABLED,
                format!(
                    "{}:computer",
                    crate::core::runtime::components::COMPONENT_DISABLED
                ),
            )));
        }
        let raw: serde_json::Value = self
            .client
            .call(MODULE, member, args)
            .await
            .map_err(AccessibilityError::Bus)?;
        decode(raw)
    }

    /// Whether `[components] computer` lets this client reach the module.
    #[must_use]
    pub fn computer_enabled(&self) -> bool {
        self.computer_enabled
    }

    /// Read native states without requesting permission.
    pub async fn permissions(&self) -> Result<PermissionStatus, AccessibilityError> {
        self.call(methods::ACCESSIBILITY_PERMISSIONS, ()).await
    }

    /// Request only the permission authorized by the host.
    pub async fn request_permission(
        &self,
        kind: PermissionKind,
    ) -> Result<PermissionStatus, AccessibilityError> {
        self.call(methods::ACCESSIBILITY_REQUEST_PERMISSION, (kind,))
            .await
    }
}

fn decode<T: DeserializeOwned>(raw: serde_json::Value) -> Result<T, AccessibilityError> {
    let reply: DesktopResponse = serde_json::from_value(raw.clone()).map_err(|_| malformed())?;
    if reply.version != tinycomputer_bus::ENVELOPE_VERSION
        || reply.ok != raw.get("data").is_some()
        || reply.ok == reply.error.is_some()
    {
        return Err(malformed());
    }
    if let Some(error) = reply.error {
        return Err(AccessibilityError::Operation(error));
    }
    serde_json::from_value(raw.get("data").cloned().ok_or_else(malformed)?).map_err(|_| malformed())
}

fn malformed() -> AccessibilityError {
    crate::modules::report::malformed_reply(MODULE);
    AccessibilityError::Bus(ModuleCallError::ModuleFault)
}

#[cfg(test)]
#[path = "accessibility_tests.rs"]
mod tests;
