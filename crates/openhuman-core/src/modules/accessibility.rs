//! Native permissions, focus and listener operations through TinyComputer.

use serde::{de::DeserializeOwned, Serialize};
use tinycomputer_bus::{names::accessibility as methods, DesktopError, DesktopResponse};

use crate::config::Config;
use tinybus::module::{ModuleCallError, ModuleClient};

pub use tinycomputer_bus::accessibility::{
    FocusQuery, FocusTarget, FocusedTextContext, GlobeBatch, GlobeHandle, GlobeHotkeyStatus,
    GlobeRead, GlobeStarted, PasteRequest, PermissionKind, PermissionState, PermissionStatus,
};

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
        confidential: bool,
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
        let raw: serde_json::Value = if confidential {
            self.client.call_confidential(MODULE, member, args).await
        } else {
            self.client.call(MODULE, member, args).await
        }
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
        self.call(methods::ACCESSIBILITY_PERMISSIONS, (), false)
            .await
    }

    /// Request only the permission authorized by the host.
    pub async fn request_permission(
        &self,
        kind: PermissionKind,
    ) -> Result<PermissionStatus, AccessibilityError> {
        self.call(methods::ACCESSIBILITY_REQUEST_PERMISSION, (kind,), false)
            .await
    }

    /// Read focused text through confidential transport.
    pub async fn focus(&self, query: FocusQuery) -> Result<FocusedTextContext, AccessibilityError> {
        self.call(methods::ACCESSIBILITY_FOCUS, (query,), true)
            .await
    }

    /// Ask the native module to validate the captured target.
    pub async fn validate_target(&self, target: FocusTarget) -> Result<(), AccessibilityError> {
        self.call(methods::ACCESSIBILITY_VALIDATE_TARGET, (target,), true)
            .await
    }

    /// Insert authorized text after native target validation.
    pub async fn paste(&self, request: PasteRequest) -> Result<(), AccessibilityError> {
        self.call(methods::ACCESSIBILITY_PASTE, (request,), true)
            .await
    }

    /// Start or recover the module's one listener lease.
    pub async fn globe_start(&self) -> Result<GlobeStarted, AccessibilityError> {
        self.call(methods::GLOBE_START, (), false).await
    }

    /// Read or acknowledge a bounded, replayable physical-key batch.
    pub async fn globe_read(&self, request: GlobeRead) -> Result<GlobeBatch, AccessibilityError> {
        self.call(methods::GLOBE_READ, (request,), false).await
    }

    /// Release a lease. A failed close retains module ownership for retry.
    pub async fn globe_stop(
        &self,
        handle: GlobeHandle,
    ) -> Result<GlobeHotkeyStatus, AccessibilityError> {
        self.call(methods::GLOBE_STOP, (handle,), false).await
    }

    /// Join native cleanup and permanently close listener admission at shutdown.
    pub async fn globe_shutdown(&self) -> Result<GlobeHotkeyStatus, AccessibilityError> {
        self.call(methods::GLOBE_SHUTDOWN, (), false).await
    }

    /// Apply OpenHuman's permission policy before opening an audio device.
    /// One-shot capture requests an undetermined permission; continuous capture
    /// retains its existing denied-only gate without presenting an OS prompt.
    pub async fn microphone_permission(
        &self,
        request_unknown: bool,
    ) -> Result<(), AccessibilityError> {
        let mut permission = self.permissions().await?.microphone;
        if request_unknown && permission == PermissionState::Unknown {
            permission = self
                .request_permission(PermissionKind::Microphone)
                .await?
                .microphone;
        }
        if permission == PermissionState::Denied
            || (request_unknown && permission == PermissionState::Unknown)
        {
            return Err(AccessibilityError::Operation(DesktopError::new(
                "PERMISSION_DENIED",
                microphone_denied_message(),
            )));
        }
        Ok(())
    }
}

/// OpenHuman's platform-specific microphone guidance.
fn microphone_denied_message() -> &'static str {
    if cfg!(target_os = "macos") {
        "Microphone permission denied. Grant access in System Settings > Privacy & Security > Microphone, then restart the app."
    } else if cfg!(target_os = "windows") {
        "Microphone access unavailable. Check Settings > Privacy & Security > Microphone and ensure the app is allowed. If no microphone is connected, plug one in."
    } else if cfg!(target_os = "linux") {
        "No microphone device available. Check your audio settings and ensure a microphone is connected. If running in a Flatpak sandbox, grant microphone access via Flatseal or system settings."
    } else {
        "Microphone access is not supported on this platform."
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
