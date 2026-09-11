//! Native and Docker shell runtime adapters (`RuntimeAdapter` implementations).

use crate::openhuman::agent::platform_shell;
use crate::openhuman::config::RuntimeConfig;
use std::path::{Path, PathBuf};

/// Runtime adapter — abstracts platform differences for tools that need
/// to spawn shell commands. The agent holds a boxed `dyn RuntimeAdapter`
/// so tools (shell, docker exec, etc.) can stay agnostic to the
/// deployment target.
pub trait RuntimeAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn has_shell_access(&self) -> bool;
    fn has_filesystem_access(&self) -> bool;
    fn storage_path(&self) -> PathBuf;
    fn supports_long_running(&self) -> bool;
    fn memory_budget(&self) -> u64 {
        0
    }
    fn build_shell_command(
        &self,
        command: &str,
        workspace_dir: &Path,
    ) -> anyhow::Result<tokio::process::Command>;
}

#[derive(Default)]
pub struct NativeRuntime {
    /// When true, shell-family child processes are spawned with the Windows
    /// `CREATE_NO_WINDOW` flag so no console window flashes. Sourced from
    /// `[shell] hide_window` (#3727/#3728). No effect on macOS/Linux.
    hide_window: bool,
}

impl NativeRuntime {
    /// A native runtime with default behaviour (no window suppression).
    pub const fn new() -> Self {
        Self { hide_window: false }
    }

    /// A native runtime that suppresses the Windows console window for spawned
    /// shell child processes when `hide_window` is true.
    pub const fn with_hide_window(hide_window: bool) -> Self {
        Self { hide_window }
    }
}

impl RuntimeAdapter for NativeRuntime {
    fn name(&self) -> &str {
        "native"
    }

    fn has_shell_access(&self) -> bool {
        true
    }

    fn has_filesystem_access(&self) -> bool {
        true
    }

    fn storage_path(&self) -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("openhuman")
            .join("runtime")
    }

    fn supports_long_running(&self) -> bool {
        true
    }

    fn memory_budget(&self) -> u64 {
        0
    }

    fn build_shell_command(
        &self,
        command: &str,
        workspace_dir: &Path,
    ) -> anyhow::Result<tokio::process::Command> {
        // Shell selection is shared with the sandboxed execution paths in
        // `sandbox::ops` so all three sites (native, unsandboxed, jailed)
        // pick the same platform-appropriate shell (#4705).
        let mut cmd = platform_shell::build_tokio_command(command);
        // Validate the CWD up front so a missing/bad action_dir produces an
        // actionable message naming the path, instead of an opaque OS error 267
        // (ERROR_DIRECTORY) from CreateProcessW on Windows / a raw ENOENT on
        // Unix when the process is spawned. Covers all three shell-family tools
        // (shell / node_exec / npm_exec) since they all route through here.
        // (#3353, Fix 2)
        crate::openhuman::config::ensure_usable_cwd(workspace_dir)?;
        cmd.current_dir(workspace_dir);
        // Optionally suppress the Windows console window for this child process
        // (`[shell] hide_window`). No-op when disabled and on non-Windows hosts.
        maybe_hide_window(&mut cmd, self.hide_window);
        Ok(cmd)
    }
}

/// Suppress the Windows console window for a shell child process when `hide` is
/// set, by applying the `CREATE_NO_WINDOW` creation flag (`0x08000000`). No-op
/// when `hide` is false, and on non-Windows hosts the flag does not exist so this
/// does nothing regardless. Delegates to the shared [`apply_no_window`] helper so
/// there is a single source of truth for the flag (#3727/#3728).
fn maybe_hide_window(cmd: &mut tokio::process::Command, hide: bool) {
    tracing::trace!(
        hide_window = hide,
        windows = cfg!(windows),
        "[agent][runtime] hide_window evaluated for shell child process"
    );
    if !hide {
        return;
    }
    crate::openhuman::inference::local::process_util::apply_no_window(cmd);
    #[cfg(windows)]
    tracing::debug!(
        creation_flags = "0x08000000",
        "[agent][runtime] applied CREATE_NO_WINDOW to shell child process"
    );
}

pub struct DockerRuntime {
    config: crate::openhuman::config::DockerRuntimeConfig,
}

impl DockerRuntime {
    fn new(config: crate::openhuman::config::DockerRuntimeConfig) -> Self {
        Self { config }
    }
}

impl RuntimeAdapter for DockerRuntime {
    fn name(&self) -> &str {
        "docker"
    }

    fn has_shell_access(&self) -> bool {
        true
    }

    fn has_filesystem_access(&self) -> bool {
        self.config.mount_workspace
    }

    fn storage_path(&self) -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("openhuman")
            .join("runtime")
            .join("docker")
    }

    fn supports_long_running(&self) -> bool {
        false
    }

    fn memory_budget(&self) -> u64 {
        self.config.memory_limit_mb.unwrap_or(0)
    }

    fn build_shell_command(
        &self,
        command: &str,
        workspace_dir: &Path,
    ) -> anyhow::Result<tokio::process::Command> {
        let workspace = workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| workspace_dir.to_path_buf());
        let mut cmd = tokio::process::Command::new("docker");
        cmd.arg("run").arg("--rm");
        cmd.arg("--network").arg(&self.config.network);

        if let Some(memory_limit_mb) = self.config.memory_limit_mb {
            cmd.arg("-m").arg(format!("{memory_limit_mb}m"));
        }
        if let Some(cpu_limit) = self.config.cpu_limit {
            cmd.arg("--cpus").arg(cpu_limit.to_string());
        }
        if self.config.read_only_rootfs {
            cmd.arg("--read-only");
        }
        if self.config.mount_workspace {
            let mount = format!("{}:/workspace", workspace.display());
            cmd.arg("-v").arg(mount);
            cmd.arg("-w").arg("/workspace");
        }

        cmd.arg(&self.config.image);
        // No `pipefail` wrapper here (unlike NativeRuntime): the in-container
        // shell is image-dependent (busybox/dash/bash) so we can't assume
        // `set -o pipefail` is supported. Container commands keep POSIX `sh`.
        cmd.arg("sh").arg("-lc").arg(command);
        Ok(cmd)
    }
}

/// Build the runtime adapter for the configured `kind`. `hide_window` comes
/// from `[shell] hide_window` and only affects the native runtime on Windows
/// (the docker runtime spawns the `docker` client, out of scope here).
pub fn create_runtime(
    config: &RuntimeConfig,
    hide_window: bool,
) -> anyhow::Result<Box<dyn RuntimeAdapter>> {
    match config.kind.as_str() {
        "native" => Ok(Box::new(NativeRuntime::with_hide_window(hide_window))),
        "docker" => Ok(Box::new(DockerRuntime::new(config.docker.clone()))),
        other => anyhow::bail!("Unsupported runtime kind: {other}"),
    }
}

#[cfg(test)]
#[path = "host_runtime_tests.rs"]
mod tests;
