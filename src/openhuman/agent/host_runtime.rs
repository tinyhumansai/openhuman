//! Native and Docker shell runtime adapters (`RuntimeAdapter` implementations).

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::{DockerRuntimeConfig, RuntimeConfig};

    #[test]
    fn native_runtime_reports_capabilities_and_shell_command() {
        let runtime = NativeRuntime::new();
        assert_eq!(runtime.name(), "native");
        assert!(runtime.has_shell_access());
        assert!(runtime.has_filesystem_access());
        assert!(runtime.supports_long_running());
        assert_eq!(runtime.memory_budget(), 0);
        assert!(runtime.storage_path().ends_with("openhuman/runtime"));

        let command = runtime
            .build_shell_command("echo hi", Path::new("/tmp"))
            .unwrap();
        let args: Vec<String> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(command.as_std().get_program().to_string_lossy(), "sh");
        assert_eq!(args, vec!["-lc", "echo hi"]);
        assert_eq!(command.as_std().get_current_dir(), Some(Path::new("/tmp")));
    }

    #[test]
    fn docker_runtime_builds_expected_flags() {
        let runtime = DockerRuntime::new(DockerRuntimeConfig {
            image: "alpine:3.20".into(),
            network: "host".into(),
            mount_workspace: true,
            read_only_rootfs: true,
            memory_limit_mb: Some(512),
            cpu_limit: Some(1.5),
            ..DockerRuntimeConfig::default()
        });
        assert_eq!(runtime.name(), "docker");
        assert!(runtime.has_shell_access());
        assert!(runtime.has_filesystem_access());
        assert!(!runtime.supports_long_running());
        assert_eq!(runtime.memory_budget(), 512);
        assert!(runtime.storage_path().ends_with("openhuman/runtime/docker"));

        let tempdir = tempfile::tempdir().unwrap();
        let command = runtime
            .build_shell_command("pwd", tempdir.path())
            .unwrap();
        let args: Vec<String> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let joined = args.join(" ");
        assert!(joined.contains("run --rm"));
        assert!(joined.contains("--network host"));
        assert!(joined.contains("-m 512m"));
        assert!(joined.contains("--cpus 1.5"));
        assert!(joined.contains("--read-only"));
        assert!(joined.contains(":/workspace"));
        assert!(joined.contains("-w /workspace"));
        assert!(joined.contains("alpine:3.20"));
        assert!(joined.ends_with("sh -lc pwd"));
    }

    #[test]
    fn create_runtime_supports_native_and_docker_and_rejects_unknown() {
        let native = create_runtime(&RuntimeConfig::default()).unwrap();
        assert_eq!(native.name(), "native");

        let docker = create_runtime(&RuntimeConfig {
            kind: "docker".into(),
            docker: DockerRuntimeConfig::default(),
            ..RuntimeConfig::default()
        })
        .unwrap();
        assert_eq!(docker.name(), "docker");

        let err = create_runtime(&RuntimeConfig {
            kind: "vm".into(),
            ..RuntimeConfig::default()
        })
        .err()
        .unwrap();
        assert!(err.to_string().contains("Unsupported runtime kind: vm"));
    }
}
