//! Sandbox domain types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Which sandbox backend to use for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxBackendKind {
    /// No sandbox — commands execute directly on the host.
    #[default]
    None,
    /// OS-level process jail via `cwd_jail` (Landlock/Seatbelt/AppContainer).
    Local,
    /// Docker container isolation.
    Docker,
}

/// Per-session sandbox policy resolved from agent definition, session
/// origin, and global config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxPolicy {
    /// Which backend to use.
    pub backend: SandboxBackendKind,
    /// Workspace root mounted into the sandbox (read/write).
    pub workspace_root: PathBuf,
    /// Additional read-only mounts (e.g. `/usr/lib`, managed node).
    pub read_only_mounts: Vec<PathBuf>,
    /// Whether outbound network is allowed inside the sandbox.
    pub allow_network: bool,
    /// Environment variables to passthrough into the sandbox.
    pub env_passthrough: Vec<String>,
    /// Docker-specific overrides (image, resource limits).
    pub docker_overrides: Option<DockerOverrides>,
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
    pub env: HashMap<String, String>,
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
mod tests {
    use super::*;

    #[test]
    fn sandbox_exec_result_success_checks() {
        let ok = SandboxExecResult {
            exit_code: 0,
            stdout: "hello".into(),
            stderr: String::new(),
            timed_out: false,
        };
        assert!(ok.success());

        let failed = SandboxExecResult {
            exit_code: 1,
            stdout: String::new(),
            stderr: "error".into(),
            timed_out: false,
        };
        assert!(!failed.success());

        let timeout = SandboxExecResult {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: true,
        };
        assert!(!timeout.success());
    }

    #[test]
    fn sandbox_backend_kind_default_is_none() {
        assert_eq!(SandboxBackendKind::default(), SandboxBackendKind::None);
    }

    #[test]
    fn sandbox_policy_serializes_roundtrip() {
        let policy = SandboxPolicy {
            backend: SandboxBackendKind::Docker,
            workspace_root: PathBuf::from("/workspace"),
            read_only_mounts: vec![PathBuf::from("/usr/lib")],
            allow_network: false,
            env_passthrough: vec!["PATH".into()],
            docker_overrides: Some(DockerOverrides {
                image: Some("node:20-slim".into()),
                memory_limit_mb: Some(256),
                ..DockerOverrides::default()
            }),
        };
        let json = serde_json::to_string(&policy).unwrap();
        let back: SandboxPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(back.backend, SandboxBackendKind::Docker);
        assert!(!back.allow_network);
    }

    #[test]
    fn elevated_tools_contains_expected_entries() {
        assert!(ELEVATED_TOOLS.contains(&"git_operations"));
        assert!(ELEVATED_TOOLS.contains(&"install_tool"));
        assert!(!ELEVATED_TOOLS.contains(&"shell"));
    }

    #[test]
    fn sandbox_status_variants() {
        assert_ne!(SandboxStatus::Inactive, SandboxStatus::Ready);
        assert_ne!(SandboxStatus::Ready, SandboxStatus::Busy);
        assert_ne!(SandboxStatus::Busy, SandboxStatus::Error);
    }
}
