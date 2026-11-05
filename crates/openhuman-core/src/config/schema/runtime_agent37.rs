//! `[runtime.agent37]` — how the agent37 sandbox backend creates its boxes.
//!
//! With `[security.sandbox] backend = "agent37"`, every sandboxed shell command
//! runs in a box the TinyBox module creates on the hosted agent37 service. The
//! service starts a box from a registered *template name*, not an OCI image
//! reference, so this is a separate setting from `[runtime.docker] image`:
//! agent37 rejects an inline image reference (for example the `alpine:3.20`
//! Docker default) with an HTTP 400, so forwarding the Docker image could never
//! create a box.
//!
//! CPU and memory are configured here too, rather than borrowed from
//! `[runtime.docker]`. agent37 offers only fixed instance shapes — `(2, 4)`,
//! `(4, 8)`, `(4, 16)` and `(4, 32)` vCPU/GB — and snaps a request up to the
//! smallest shape that covers both bounds. CPU above 4 vCPU is capped at 4
//! before a shape is chosen, so it does not raise the memory the box gets;
//! memory above 32 GiB gets the largest shape. These settings are therefore a
//! request, not a hard ceiling or floor, which is why the sandbox reports them
//! as best effort. The defaults ask for agent37's smallest shape.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One mebibyte, for converting [`Agent37RuntimeConfig::memory_mb`] to bytes.
const MEBIBYTE: u64 = 1024 * 1024;

/// The agent37 template a box starts from when none is configured. It is a
/// template name the service already knows, not an OCI image reference.
pub const DEFAULT_AGENT37_TEMPLATE: &str = "agent37-base";

/// Default CPU a box asks for, in thousandths of a core: 2 cores, the vCPU
/// count of agent37's smallest instance shape.
pub const DEFAULT_AGENT37_CPU_MILLIS: u32 = 2_000;

/// Default memory a box asks for, in mebibytes: 4 GiB, the memory of agent37's
/// smallest instance shape.
pub const DEFAULT_AGENT37_MEMORY_MB: u64 = 4_096;

/// CPU of agent37's largest instance shape, in thousandths of a core.
pub const MAX_AGENT37_CPU_MILLIS: u32 = 4_000;

/// Memory of agent37's largest instance shape, in mebibytes: 32 GiB.
pub const MAX_AGENT37_MEMORY_MB: u64 = 32_768;

/// `[runtime.agent37]`: what each agent37 box is created from and with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Agent37RuntimeConfig {
    /// The registered agent37 template name every box starts from. This names a
    /// template the service hosts; it is not an OCI image reference.
    pub template: String,
    /// CPU a box asks for, in thousandths of a core. agent37 offers only 2- or
    /// 4-vCPU shapes and snaps the request up to one, capping it at 4 vCPU, so
    /// this is best effort.
    pub cpu_millis: u32,
    /// Memory a box asks for, in mebibytes. agent37 offers only fixed shapes
    /// and snaps the request up to the smallest that covers it, capping it at
    /// 32 GiB, so this is best effort.
    pub memory_mb: u64,
}

impl Agent37RuntimeConfig {
    /// [`Self::memory_mb`] in bytes, always a whole number of mebibytes, or
    /// `None` when that many bytes does not fit in a `u64`.
    pub fn memory_bytes(&self) -> Option<u64> {
        self.memory_mb.checked_mul(MEBIBYTE)
    }

    /// Whether this asks for more than agent37's largest shape, so the box will
    /// get less than was configured.
    pub fn exceeds_largest_shape(&self) -> bool {
        self.cpu_millis > MAX_AGENT37_CPU_MILLIS || self.memory_mb > MAX_AGENT37_MEMORY_MB
    }
}

impl Default for Agent37RuntimeConfig {
    fn default() -> Self {
        Self {
            template: DEFAULT_AGENT37_TEMPLATE.to_owned(),
            cpu_millis: DEFAULT_AGENT37_CPU_MILLIS,
            memory_mb: DEFAULT_AGENT37_MEMORY_MB,
        }
    }
}

#[cfg(test)]
#[path = "runtime_agent37_tests.rs"]
mod tests;
