use super::*;
use crate::agent::host_runtime::{NativeRuntime, RuntimeAdapter};
use crate::security::policy::native_commands_stub::StubGateway;
use crate::security::policy::{CommandFlow, CommandStage, Refusal};
use crate::security::{AutonomyLevel, SecurityPolicy};

/// A policy whose command gateway allows everything, so a test of the tool's
/// own behaviour (execution, timeouts, environment, audit) does not depend on
/// loading the security module. What the module answers for a command is pinned
/// by the module-backed tests in `modules/security_command_tests.rs`.
fn test_security(autonomy: AutonomyLevel) -> Arc<SecurityPolicy> {
    gated_security(autonomy, StubGateway::allow())
}

fn gated_security(autonomy: AutonomyLevel, gateway: Arc<StubGateway>) -> Arc<SecurityPolicy> {
    Arc::new(
        SecurityPolicy {
            autonomy,
            workspace_dir: std::env::temp_dir(),
            action_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        }
        .with_gateway(gateway),
    )
}

fn test_runtime() -> Arc<dyn RuntimeAdapter> {
    Arc::new(NativeRuntime::new())
}

fn test_audit() -> Arc<AuditLogger> {
    AuditLogger::disabled()
}

fn audit_with_tempdir() -> (Arc<AuditLogger>, tempfile::TempDir) {
    use crate::config::AuditConfig;
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
    use tinytools::WorkspaceDescriptor;
    let ws = WorkspaceDescriptor::new(root.to_path_buf()).with_policy_id("test-worktree");
    let ctx: RunContext = RunContext::new(RunConfig::new("test-run"), ()).with_workspace(ws);
    ToolExecutionContext::from_run_context(&ctx, tinyagents_harness::ids::CallId::new("test-call"))
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

#[path = "shell_tests_platform_tests.rs"]
mod platform_tests;
#[path = "shell_tests_runtime_and_sandbox_tests.rs"]
mod runtime_and_sandbox_tests;
#[path = "shell_tests_schema_and_env_tests.rs"]
mod schema_and_env_tests;
#[path = "shell_tests_timeout_tests.rs"]
mod timeout_tests;
