use std::path::{Path, PathBuf};

/// Runtime adapter — abstracts platform differences so the same agent
/// code runs on native, Docker, Cloudflare Workers, Raspberry Pi, etc.
pub trait RuntimeAdapter: Send + Sync {
    /// Human-readable runtime name
    fn name(&self) -> &str;

    /// Whether this runtime supports shell access
    fn has_shell_access(&self) -> bool;

    /// Whether this runtime supports filesystem access
    fn has_filesystem_access(&self) -> bool;

    /// Base storage path for this runtime
    fn storage_path(&self) -> PathBuf;

    /// Whether long-running processes (gateway, heartbeat) are supported
    fn supports_long_running(&self) -> bool;

    /// Maximum memory budget in bytes (0 = unlimited)
    fn memory_budget(&self) -> u64 {
        0
    }

    /// Build a shell command process for this runtime.
    fn build_shell_command(
        &self,
        command: &str,
        workspace_dir: &Path,
    ) -> anyhow::Result<tokio::process::Command>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyRuntime;

    impl RuntimeAdapter for DummyRuntime {
        fn name(&self) -> &str {
            "dummy-runtime"
        }

        fn has_shell_access(&self) -> bool {
            true
        }

        fn has_filesystem_access(&self) -> bool {
            true
        }

        fn storage_path(&self) -> PathBuf {
            PathBuf::from("/tmp/dummy-runtime")
        }

        fn supports_long_running(&self) -> bool {
            true
        }

        fn build_shell_command(
            &self,
            command: &str,
            workspace_dir: &Path,
        ) -> anyhow::Result<tokio::process::Command> {
            let mut cmd = tokio::process::Command::new("echo");
            cmd.arg(command);
            cmd.current_dir(workspace_dir);
            Ok(cmd)
        }
    }

    #[test]
    fn default_memory_budget_is_zero() {
        let runtime = DummyRuntime;
        assert_eq!(runtime.memory_budget(), 0);
    }

    #[test]
    fn runtime_reports_capabilities() {
        let runtime = DummyRuntime;

        assert_eq!(runtime.name(), "dummy-runtime");
        assert!(runtime.has_shell_access());
        assert!(runtime.has_filesystem_access());
        assert!(runtime.supports_long_running());
        assert_eq!(runtime.storage_path(), PathBuf::from("/tmp/dummy-runtime"));
    }

    #[tokio::test]
    async fn build_shell_command_executes() {
        let runtime = DummyRuntime;
        let mut cmd = runtime
            .build_shell_command("hello-runtime", Path::new("."))
            .unwrap();

        let output = cmd.output().await.unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(output.status.success());
        assert!(stdout.contains("hello-runtime"));
    }
}
