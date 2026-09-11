use super::*;
use crate::openhuman::agent::host_runtime::{NativeRuntime, RuntimeAdapter};
use crate::openhuman::security::{AutonomyLevel, CommandClass, SecurityPolicy};

fn test_security(autonomy: AutonomyLevel) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy,
        workspace_dir: std::env::temp_dir(),
        action_dir: std::env::temp_dir(),
        ..SecurityPolicy::default()
    })
}

fn test_runtime() -> Arc<dyn RuntimeAdapter> {
    Arc::new(NativeRuntime::new())
}

fn test_audit() -> Arc<AuditLogger> {
    AuditLogger::disabled()
}

fn audit_with_tempdir() -> (Arc<AuditLogger>, tempfile::TempDir) {
    use crate::openhuman::config::AuditConfig;
    let tmp = tempfile::tempdir().expect("create tempdir");
    let logger = AuditLogger::new(
        AuditConfig {
            enabled: true,
            log_path: "audit.log".into(),
            max_size_mb: 10,
        },
        tmp.path().to_path_buf(),
    )
    .expect("create audit logger");
    (Arc::new(logger), tmp)
}

/// Build a `ToolExecutionContext` carrying a `WorkspaceDescriptor` rooted
/// at `root`, mirroring what the tinyagents harness threads into every tool
/// call of a worktree-isolated worker (`RunContext::with_workspace` →
/// `ToolExecutionContext::from_run_context`).
fn tool_context_with_workspace(
    root: &std::path::Path,
) -> tinyagents_harness::tool::ToolExecutionContext {
    use tinyagents_harness::context::{RunConfig, RunContext};
    use tinyagents_harness::tool::ToolExecutionContext;
    use tinyagents_harness::workspace::WorkspaceDescriptor;
    let ws = WorkspaceDescriptor::new(root.to_path_buf()).with_policy_id("test-worktree");
    let ctx: RunContext = RunContext::new(RunConfig::new("test-run"), ()).with_workspace(ws);
    ToolExecutionContext::from_run_context(&ctx)
}

fn test_security_with_env_cmd() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        workspace_dir: std::env::temp_dir(),
        action_dir: std::env::temp_dir(),
        allowed_commands: vec!["echo".into(), "mkdir".into()],
        ..SecurityPolicy::default()
    })
}

/// RAII guard that restores an environment variable to its original state on drop,
/// ensuring cleanup even if the test panics.
struct EnvGuard {
    key: &'static str,
    original: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let original = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, original }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(val) => std::env::set_var(self.key, val),
            None => std::env::remove_var(self.key),
        }
    }
}

#[path = "shell_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "shell_tests_part_02_tests.rs"]
mod part_02_tests;
