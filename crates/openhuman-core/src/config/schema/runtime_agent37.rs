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
//!
//! Each tenant keeps one box between commands, so the files it makes are
//! there for the next command, even after this process restarts. A box falls
//! asleep after `idle_sleep_secs` without a command (clamped to 300–86400, the
//! range agent37 accepts) and wakes on the next one; it is deleted once it has
//! slept for `delete_after_secs`. `max_live` caps the boxes this process holds
//! a handle on at once; letting go of a handle leaves the box asleep, not
//! gone. The sleep delay and the cap are read when the first box is needed,
//! so changing them takes effect on the next core start. Hosts sharing one
//! agent37 workspace should agree on `delete_after_secs`: expiry applies it to
//! every retained box the service lists.
//!
//! A retained box is found again by an opaque key hashed from the tenant, the
//! active workspace and `retention_namespace`. Deployments that share one
//! agent37 key and could see equal tenants in equal workspace paths (staging
//! and production of one service, say) must each set their own
//! `retention_namespace`; left blank, the workspace alone keeps hosts and
//! signed-in users apart. Changing it (or the workspace) starts each tenant
//! on a fresh box; the old one sleeps until `delete_after_secs` deletes it.

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

/// Default seconds a box may sit idle before it falls asleep: agent37's own
/// default of 15 minutes.
pub const DEFAULT_AGENT37_IDLE_SLEEP_SECS: u32 = 900;

/// Default seconds a box may sleep before it is deleted: 30 days.
pub const DEFAULT_AGENT37_DELETE_AFTER_SECS: u64 = 30 * 24 * 60 * 60;

/// Default most boxes this process holds a handle on at once.
pub const DEFAULT_AGENT37_MAX_LIVE: usize = 50;

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
    /// Seconds a box may sit idle before it falls asleep (it wakes on the next
    /// command). Clamped to 300–86400, the range agent37 accepts.
    pub idle_sleep_secs: u32,
    /// Seconds a box may stay asleep before it is deleted, with its files.
    /// Zero is raised to 1.
    pub delete_after_secs: u64,
    /// The most boxes this process holds a handle on at once. A box let go of
    /// to make room is left asleep, not deleted.
    pub max_live: usize,
    /// Names this deployment among those sharing one agent37 key, so equal
    /// tenants of two deployments never adopt each other's retained boxes.
    /// Only its hash is used; blank (the default) scopes boxes by the active
    /// workspace alone.
    pub retention_namespace: String,
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
            idle_sleep_secs: DEFAULT_AGENT37_IDLE_SLEEP_SECS,
            delete_after_secs: DEFAULT_AGENT37_DELETE_AFTER_SECS,
            max_live: DEFAULT_AGENT37_MAX_LIVE,
            retention_namespace: String::new(),
        }
    }
}

#[cfg(test)]
#[path = "runtime_agent37_tests.rs"]
mod tests;
