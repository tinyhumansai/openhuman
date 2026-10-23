//! Sandbox domain types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// The executor an operator or a deployment asks the security module to plan onto.
///
/// This is a request, never a decision: the module picks the executor from the ones
/// the host verified and may refuse (`sandbox/spawn_plan/`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxBackendKind {
    /// No preference: the module plans onto the first verified executor that meets
    /// the spawn's isolation floor.
    #[default]
    None,
    /// The module-owned native process jail selected by TinyBox.
    Local,
    /// Docker container isolation.
    Docker,
}

/// Per-session sandbox policy resolved from agent definition, session
/// origin, and global config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxPolicy {
    /// The executor the module is required to plan onto, or none for the module's own
    /// choice.
    pub backend: SandboxBackendKind,
    /// Workspace root mounted into the sandbox (read/write).
    pub workspace_root: PathBuf,
    /// The core's own state directory (`workspace_dir`), never the user's
    /// project. Per-call scratch the sandbox needs on the host, such as the
    /// local jail's output capture, lives under it so nothing appears in
    /// `workspace_root` while a command runs.
    pub state_dir: PathBuf,
    /// Additional read-only mounts (e.g. `/usr/lib`).
    pub read_only_mounts: Vec<PathBuf>,
    /// Additional read-write mounts (e.g. `~/.cargo`). Local jail only.
    #[serde(default)]
    pub read_write_mounts: Vec<PathBuf>,
    /// Whether outbound network is allowed inside the sandbox.
    pub allow_network: bool,
    /// Environment variables to passthrough into the sandbox.
    pub env_passthrough: Vec<String>,
    /// Docker-specific overrides (image, resource limits).
    pub docker_overrides: Option<DockerOverrides>,
    /// Whether a container mounts `workspace_root`. `false` asks the module for a plan
    /// with no workspace grant (`[runtime.docker] mount_workspace = false`).
    #[serde(default = "mount_workspace_default")]
    pub mount_workspace: bool,
}

fn mount_workspace_default() -> bool {
    true
}

/// Docker-specific sandbox overrides layered on top of the global
/// `DockerRuntimeConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DockerOverrides {
    pub image: Option<String>,
    pub network: Option<String>,
    pub memory_limit_mb: Option<u64>,
    pub cpu_limit: Option<f64>,
    pub read_only_rootfs: Option<bool>,
    pub extra_caps_drop: Vec<String>,
}

/// A request to execute a command inside a sandbox.
#[derive(Debug, Clone)]
pub struct SandboxExecRequest {
    /// Shell command string to execute.
    pub command: String,
    /// Working directory inside the sandbox. For Docker this is the
    /// container-side path (e.g. `/workspace`).
    pub working_dir: PathBuf,
    /// Environment variables to inject.
    pub env: HashMap<std::ffi::OsString, std::ffi::OsString>,
    /// Execution timeout.
    pub timeout: std::time::Duration,
}

/// Result of a sandboxed command execution.
#[derive(Debug, Clone)]
pub struct SandboxExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    /// Whether the command was killed due to timeout.
    pub timed_out: bool,
}

impl SandboxExecResult {
    pub fn success(&self) -> bool {
        self.exit_code == 0 && !self.timed_out
    }
}

/// Handle to an active sandbox backend. Callers use this to execute
/// commands and query status without knowing the backend implementation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxBackendHandle {
    pub kind: SandboxBackendKind,
    pub status: SandboxStatus,
    /// Container ID for Docker backends; jail label for local.
    pub backend_id: Option<String>,
    /// How strongly a ready local jail confines commands; `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<SandboxIsolation>,
}

impl SandboxBackendHandle {
    /// Whether this backend meets the isolation floor for untrusted code: a
    /// ready container, or a ready local jail with kernel-level isolation.
    /// The unsandboxed backend, a folder-confined (Seatbelt) jail and anything
    /// not ready never do, so callers refuse instead of running unconfined.
    pub fn suitable_for_untrusted_code(&self) -> bool {
        if self.status != SandboxStatus::Ready {
            return false;
        }
        match self.kind {
            SandboxBackendKind::Docker => true,
            SandboxBackendKind::Local => self.isolation == Some(SandboxIsolation::Isolated),
            SandboxBackendKind::None => false,
        }
    }
}

/// `sandbox.status` payload: the backend handle plus the untrusted-code floor,
/// so a UI can tell "ready" from "ready for untrusted code".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxStatusReport {
    #[serde(flatten)]
    pub handle: SandboxBackendHandle,
    /// See [`SandboxBackendHandle::suitable_for_untrusted_code`].
    pub suitable_for_untrusted_code: bool,
}

impl SandboxStatusReport {
    pub fn new(handle: SandboxBackendHandle) -> Self {
        let suitable_for_untrusted_code = handle.suitable_for_untrusted_code();
        Self {
            handle,
            suitable_for_untrusted_code,
        }
    }
}

/// The confinement a local native jail actually provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxIsolation {
    /// Kernel (or stronger) process isolation: fit for untrusted code.
    Isolated,
    /// Writes confined to the selected folder (macOS Seatbelt); no process
    /// isolation.
    FolderConfined,
}

/// Health/lifecycle status of a sandbox backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxStatus {
    /// Backend not initialized.
    Inactive,
    /// Backend ready to accept commands.
    Ready,
    /// Backend is executing a command.
    Busy,
    /// Backend encountered an error and needs restart/cleanup.
    Error,
}

/// Operations that explicitly require host-level access and cannot run
/// inside a sandbox. The elevated path is audited and requires the
/// caller to declare the reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElevatedOp {
    /// The tool or operation name requesting elevation.
    pub tool_name: String,
    /// Human-readable reason this operation needs host access.
    pub reason: String,
    /// The command or action to execute on the host.
    pub command: String,
}

/// Well-known tool names that always require host access.
pub const ELEVATED_TOOLS: &[&str] = &[
    "git_operations",
    "install_tool",
    "docker_management",
    "process_management",
];

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
