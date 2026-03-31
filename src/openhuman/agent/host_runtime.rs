//! Native and Docker shell runtime adapters (`RuntimeAdapter` implementations).

use crate::openhuman::config::RuntimeConfig;
use std::path::{Path, PathBuf};

pub use crate::openhuman::agent::traits::RuntimeAdapter;

pub struct NativeRuntime;

impl Default for NativeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeRuntime {
    pub const fn new() -> Self {
        Self
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
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-lc").arg(command).current_dir(workspace_dir);
        Ok(cmd)
    }
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
        cmd.arg("sh").arg("-lc").arg(command);
        Ok(cmd)
    }
}

pub fn create_runtime(config: &RuntimeConfig) -> anyhow::Result<Box<dyn RuntimeAdapter>> {
    match config.kind.as_str() {
        "native" => Ok(Box::new(NativeRuntime::new())),
        "docker" => Ok(Box::new(DockerRuntime::new(config.docker.clone()))),
        other => anyhow::bail!("Unsupported runtime kind: {other}"),
    }
}
