use super::*;
use crate::modules::tinybox::host_runtime::{NativeRuntime, RuntimeAdapter};
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
    Arc::new(
        SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: std::env::temp_dir(),
            action_dir: std::env::temp_dir(),
            allowed_commands: vec!["echo".into(), "mkdir".into()],
            ..SecurityPolicy::default()
        }
        .with_gateway(StubGateway::allow()),
    )
}

#[path = "shell_tests_keep_running_tests.rs"]
mod keep_running_tests;
#[path = "shell_tests_platform_tests.rs"]
mod platform_tests;
#[path = "shell_tests_runtime_and_sandbox_tests.rs"]
mod runtime_and_sandbox_tests;
#[path = "shell_tests_schema_and_env_tests.rs"]
mod schema_and_env_tests;
#[path = "shell_tests_timeout_tests.rs"]
mod timeout_tests;

/// The shell tool's gate on the released module, with no gateway stub: a read
/// runs in read-only mode, a write and hidden execution are refused before
/// anything is launched, and the run spends the module's hourly budget.
#[cfg(feature = "security-module")]
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn shell_tool_gates_commands_on_the_released_module() {
    use crate::modules::security::released_support::Released;

    let released = Released::new();
    let ctx = DispatchCtx::ambient();
    let tool = |autonomy, budget| {
        ShellTool::new(Arc::new(released.policy(autonomy, budget)), test_runtime())
    };

    let read_only = tool(AutonomyLevel::ReadOnly, 100);
    let (allowed, result) = read_only.run_with_security(&ctx, "ls", None).await;
    assert!(allowed, "a read runs in read-only mode: {result:?}");
    let (allowed, result) = read_only
        .run_with_security(&ctx, "touch refused", None)
        .await;
    assert!(!allowed, "a write is refused in read-only mode");
    assert!(
        format!("{:?}", result.content).contains(tinytools::status::POLICY_BLOCKED_MARKER),
        "{:?}",
        result.content
    );
    assert!(!released.action.join("refused").exists(), "nothing ran");

    let supervised = tool(AutonomyLevel::Supervised, 100);
    let (allowed, result) = supervised
        .run_with_security(&ctx, "echo $(whoami)", None)
        .await;
    assert!(!allowed, "hidden execution is refused");
    assert!(!format!("{:?}", result.content).contains("whoami"));

    let limited = tool(AutonomyLevel::Supervised, 1);
    assert!(limited.run_with_security(&ctx, "ls", None).await.0);
    let (allowed, result) = limited.run_with_security(&ctx, "ls", None).await;
    assert!(!allowed, "the module's hourly budget is spent by the run");
    assert!(format!("{:?}", result.content).contains("Rate limit exceeded"));
}
