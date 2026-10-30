//! `[runtime.openshell]` — the boxes the OpenShell sandbox backend creates.
//!
//! With `[security.sandbox] backend = "openshell"`, every sandboxed shell
//! command runs in a box the TinyBox module creates on an NVIDIA OpenShell
//! gateway. Each tenant (profile and agent) gets one box, created on its first
//! command, reused after that, and closed once it has been idle for
//! `idle_ttl_secs`. `max_live` caps the boxes alive at once across the
//! process. The idle TTL and the cap are read when the first box is needed, so
//! changing them takes effect on the next core start; the image and limits
//! apply to every box created after a change.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The image a box starts from when none is configured: the OpenShell
/// community base sandbox, which carries the `/sandbox` workspace, `sh` and
/// `timeout` the backend relies on.
pub const DEFAULT_OPENSHELL_IMAGE: &str =
    "ghcr.io/nvidia/openshell-community/sandboxes/base:latest";

const MEBIBYTE: u64 = 1024 * 1024;

/// `[runtime.openshell]`: what each OpenShell box is created with, and how
/// many may live at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OpenShellRuntimeConfig {
    /// OCI image reference every new box starts from.
    pub image: String,
    /// CPU allowance per box, in thousandths of a core.
    pub cpu_millis: u32,
    /// Memory per box, in mebibytes. OpenShell only accepts whole mebibytes,
    /// which is why the setting is not in bytes.
    pub memory_mb: u64,
    /// Seconds a box may sit idle before it is closed.
    pub idle_ttl_secs: u64,
    /// The most boxes alive at once in this process.
    pub max_live: usize,
}

impl OpenShellRuntimeConfig {
    /// [`Self::memory_mb`] in bytes, always a whole number of mebibytes, or
    /// `None` when that many bytes does not fit in a `u64`.
    pub fn memory_bytes(&self) -> Option<u64> {
        self.memory_mb.checked_mul(MEBIBYTE)
    }
}

impl Default for OpenShellRuntimeConfig {
    fn default() -> Self {
        Self {
            image: DEFAULT_OPENSHELL_IMAGE.to_owned(),
            cpu_millis: 1_000,
            memory_mb: 256,
            idle_ttl_secs: 900,
            max_live: 50,
        }
    }
}

#[cfg(test)]
#[path = "runtime_openshell_tests.rs"]
mod tests;
