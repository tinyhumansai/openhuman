//! Native and Docker shell runtime adapters (`RuntimeAdapter` implementations).

use crate::config::RuntimeConfig;
use std::path::{Path, PathBuf};
pub use tinybox_bus::ShellFlavor;

/// The shell family the TinyBox module selects on this machine.
///
/// TinyBox is loaded in-process, so its machine is this host. This is only the
/// answer to "what do we tell the model about the shell?"; which shell actually
/// runs, and how, is the module's `SelectShell` / `StartNativeShellExec`.
pub const fn host_shell_flavor() -> ShellFlavor {
    if cfg!(windows) {
        ShellFlavor::Cmd
    } else {
        ShellFlavor::Posix
    }
}

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
    /// The shell family commands run under, so what the `shell` tool tells the
    /// model matches what actually executes. Defaults to the host's own shell.
    fn shell_flavor(&self) -> ShellFlavor {
        host_shell_flavor()
    }

    /// Windows: start shell children without a console window
    /// (`[shell] hide_window`). Passed to TinyBox on the native shell request.
    fn hide_window(&self) -> bool {
        false
    }

    /// Build a module-owned Docker request when this runtime does not execute
    /// through a host child process. The shell tool supplies its filtered
    /// environment and deadline before dispatching the request.
    #[cfg(feature = "modules")]
    fn build_docker_request(
        &self,
        _command: &str,
        _workspace_dir: &Path,
    ) -> Option<(crate::modules::tinybox::contract::DockerRunRequest, bool)> {
        None
    }
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

    fn hide_window(&self) -> bool {
        self.hide_window
    }
}

pub struct DockerRuntime {
    config: crate::config::DockerRuntimeConfig,
}

impl DockerRuntime {
    fn new(config: crate::config::DockerRuntimeConfig) -> Self {
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

    /// Commands run under `sh -lc` inside the container on every host,
    /// including Windows, so the model must be told POSIX, not cmd.exe.
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }

    #[cfg(feature = "modules")]
    fn build_docker_request(
        &self,
        command: &str,
        workspace_dir: &Path,
    ) -> Option<(crate::modules::tinybox::contract::DockerRunRequest, bool)> {
        let workspace = if self.config.mount_workspace {
            workspace_dir
                .canonicalize()
                .unwrap_or_else(|_| workspace_dir.to_path_buf())
                .to_string_lossy()
                .into_owned()
        } else {
            String::new()
        };
        Some((
            crate::modules::tinybox::contract::DockerRunRequest {
                workspace,
                image: Some(self.config.image.clone()),
                command: command.to_owned(),
                login_shell: true,
                network: Some(self.config.network.clone()),
                memory_limit_mb: self.config.memory_limit_mb,
                cpus: self.config.cpu_limit,
                read_only_rootfs: Some(self.config.read_only_rootfs),
                read_only_mounts: Vec::new(),
                extra_cap_drops: Vec::new(),
                env: Default::default(),
                label: Some("openhuman.runtime=true".to_owned()),
                timeout_ms: crate::modules::tinybox::contract::MAX_DOCKER_RUN_TIMEOUT_MS,
            },
            self.config.mount_workspace,
        ))
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
