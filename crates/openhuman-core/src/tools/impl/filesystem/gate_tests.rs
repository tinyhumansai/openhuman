//! The filesystem tools driven through the real [`SecurityPolicy`] adapter.
//!
//! The tools' own behavior is tested in `tinytools_std::filesystem` against a
//! fake gate. What can only be tested here is the policy semantics the adapter
//! exposes to them: the autonomy tiers, the workspace boundary, symlink
//! escapes, the action budget, the approval flag, and the per-call workspace
//! grant.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use tinytools::{Tool, ToolCallOptions, ToolRunContext, WorkspaceDescriptor};
use tinytools_std::filesystem::{
    ApplyPatchTool, CsvExportTool, EditFileTool, FileReadTool, FileWriteTool, FsGate,
    GitOperationsTool,
};

use crate::security::policy::native_commands_stub::StubGateway;
use crate::security::{AutonomyLevel, SecurityPolicy};
use tinytools::status::POLICY_BLOCKED_MARKER;

fn policy(dir: &Path, autonomy: AutonomyLevel, max_actions_per_hour: u32) -> Arc<SecurityPolicy> {
    // Class questions are answered with the tier table, so these tests need not
    // load the security module.
    Arc::new(
        SecurityPolicy {
            autonomy,
            workspace_dir: dir.to_path_buf(),
            action_dir: dir.to_path_buf(),
            max_actions_per_hour,
            ..SecurityPolicy::default()
        }
        .with_gateway(StubGateway::tiered()),
    )
}

fn supervised(dir: &Path) -> Arc<SecurityPolicy> {
    policy(dir, AutonomyLevel::Supervised, 1000)
}

/// A run context carrying an isolated workspace, as the session builder makes.
struct Workspace(WorkspaceDescriptor);

impl Workspace {
    fn at(root: &Path) -> Self {
        Self(WorkspaceDescriptor::new(root.to_path_buf()).with_policy_id("test-descriptor"))
    }
}

impl ToolRunContext for Workspace {
    fn workspace(&self) -> Option<&WorkspaceDescriptor> {
        Some(&self.0)
    }
}

#[cfg(feature = "security-module")]
use crate::modules::security::gap::stub::{Asked, StubPort};
#[cfg(feature = "security-module")]
use crate::modules::security::gap::{install_port, BudgetOp};
#[cfg(feature = "security-module")]
use tinysecurity_bus::{BudgetDecision2, PathCheckKind};

#[cfg(feature = "security-module")]
#[test]
fn adapter_reports_the_autonomy_tier() {
    let dir = tempfile::tempdir().unwrap();
    let read_only = policy(dir.path(), AutonomyLevel::ReadOnly, 10);
    {
        // The tier is the module's: read-only refuses, and a spent hourly budget
        // is not a tier refusal.
        let stub = Arc::new(StubPort::allowing().budget(|_, _| BudgetDecision2::ReadOnlyMode));
        let _module = install_port(stub.clone());
        assert!(!FsGate::can_act(read_only.as_ref()));
        assert_eq!(stub.asked(), vec![Asked::Budget(BudgetOp::Peek)]);
    }
    assert!(read_only.is_read_only());
    assert!(
        !read_only.write_needs_approval(),
        "read-only blocks, it does not prompt"
    );

    let supervised = policy(dir.path(), AutonomyLevel::Supervised, 10);
    let _module = install_port(Arc::new(StubPort::allowing()));
    assert!(FsGate::can_act(supervised.as_ref()));
    assert!(!supervised.is_read_only());
    assert!(supervised.write_needs_approval());

    let full = policy(dir.path(), AutonomyLevel::Full, 10);
    assert!(FsGate::can_act(full.as_ref()));
    assert!(!full.write_needs_approval(), "Full runs writes unprompted");

    let _module = install_port(Arc::new(StubPort::unavailable()));
    assert!(
        !FsGate::can_act(full.as_ref()),
        "an unanswered tier refuses"
    );
}

#[cfg(feature = "security-module")]
#[test]
fn the_budget_is_reserved_by_the_tool_gate_not_counted_again_in_the_tool() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(
        StubPort::allowing().budget(|_, _| BudgetDecision2::RateLimited { max_per_hour: 2 }),
    );
    let _module = install_port(stub.clone());
    let gate = policy(dir.path(), AutonomyLevel::Supervised, 2);
    assert!(!FsGate::is_rate_limited(gate.as_ref()));
    assert!(FsGate::record_action(gate.as_ref()));
    assert!(FsGate::record_action(gate.as_ref()));
    assert!(FsGate::record_action(gate.as_ref()));
    assert!(stub.asked().is_empty(), "no in-tool budget question");
}

#[cfg(feature = "security-module")]
#[test]
fn adapter_exposes_the_action_dir_and_asks_the_module_for_string_checks() {
    let dir = tempfile::tempdir().unwrap();
    let stub =
        Arc::new(StubPort::allowing().paths(|_, _, path| !path.to_string_lossy().contains("..")));
    let _module = install_port(stub.clone());
    let gate = supervised(dir.path());
    assert_eq!(FsGate::action_dir(gate.as_ref()), dir.path());
    assert!(!FsGate::is_path_string_allowed(gate.as_ref(), "../escape"));
    assert!(FsGate::is_path_string_allowed(gate.as_ref(), "inside.txt"));

    // A walk's hits are one module call, in order, not one per hit.
    let hits: Vec<String> = ["a.rs", "../b.rs", "c.rs"].map(String::from).to_vec();
    assert_eq!(
        FsGate::paths_string_allowed(gate.as_ref(), &hits),
        vec![true, false, true]
    );
    let asked = stub.asked();
    assert_eq!(asked.len(), 3);
    assert!(matches!(
        &asked[2],
        Asked::Paths(PathCheckKind::StringAllowed, paths) if paths.len() == 3
    ));

    let _module = install_port(Arc::new(StubPort::unavailable()));
    assert_eq!(
        FsGate::paths_string_allowed(gate.as_ref(), &hits),
        vec![false; 3],
        "an unanswered batch refuses every hit"
    );
}

#[tokio::test]
async fn scoped_gate_is_rooted_at_the_granted_workspace() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("main.rs"), "fn main() {}").unwrap();
    let gate: Arc<dyn FsGate> = supervised(home.path());

    assert!(gate.validate_path("main.rs").await.is_err());
    let scoped = gate.scoped_to_workspace(project.path());
    assert_eq!(scoped.action_dir(), project.path());
    assert!(scoped.validate_path("main.rs").await.is_ok());
    assert!(scoped.validate_parent_path("new.rs").await.is_ok());
    // The original gate is unchanged by the per-call grant.
    assert!(gate.validate_path("main.rs").await.is_err());
}

#[tokio::test]
async fn file_read_blocks_traversal_and_absolute_paths() {
    let dir = tempfile::tempdir().unwrap();
    let tool = FileReadTool::new(supervised(dir.path()));
    for path in ["../../../etc/passwd", "/etc/passwd"] {
        let result = tool.execute(json!({"path": path})).await.unwrap();
        assert!(result.is_error);
        assert!(
            result.output().contains("not allowed"),
            "{}",
            result.output()
        );
        assert!(result.output().contains(POLICY_BLOCKED_MARKER));
    }
}

#[tokio::test]
async fn file_read_reports_a_missing_file_as_unresolvable() {
    let dir = tempfile::tempdir().unwrap();
    let tool = FileReadTool::new(supervised(dir.path()));
    let result = tool.execute(json!({"path": "nope.txt"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Failed to resolve"));
}

#[tokio::test]
async fn file_read_is_allowed_in_read_only_mode() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "readonly ok").unwrap();
    let tool = FileReadTool::new(policy(dir.path(), AutonomyLevel::ReadOnly, 20));
    let result = tool.execute(json!({"path": "a.txt"})).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(result.output(), "readonly ok");
}

#[tokio::test]
async fn write_tools_are_blocked_in_read_only_mode_with_the_hard_reject_marker() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "old").unwrap();
    let gate = policy(dir.path(), AutonomyLevel::ReadOnly, 20);

    let write = FileWriteTool::new(gate.clone())
        .execute(json!({"path": "out.txt", "content": "nope"}))
        .await
        .unwrap();
    let edit = EditFileTool::new(gate.clone())
        .execute(json!({"path": "a.txt", "old_string": "old", "new_string": "new"}))
        .await
        .unwrap();
    let patch = ApplyPatchTool::new(gate.clone())
        .execute(json!({"edits": [{"path": "a.txt", "old_string": "old", "new_string": "new"}]}))
        .await
        .unwrap();
    let csv = CsvExportTool::new(gate)
        .execute(json!({"filename": "o.csv", "data": "[{\"a\": 1}]"}))
        .await
        .unwrap();

    for result in [write, edit, patch, csv] {
        assert!(result.is_error);
        assert!(result.output().contains("read-only"), "{}", result.output());
        assert!(
            result.output().contains(POLICY_BLOCKED_MARKER),
            "a read-only block must carry the hard-reject marker: {}",
            result.output()
        );
    }
    assert!(!dir.path().join("out.txt").exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "old"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_out_of_the_workspace_are_refused_for_reads_and_writes() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let outside = root.path().join("outside");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "secret").unwrap();
    symlink(&outside, workspace.join("escape_dir")).unwrap();

    let gate = supervised(&workspace);
    let read = FileReadTool::new(gate.clone())
        .execute(json!({"path": "escape_dir/secret.txt"}))
        .await
        .unwrap();
    assert!(read.is_error);
    assert!(!read.output().contains("secret") || read.output().contains("not allowed"));

    let write = FileWriteTool::new(gate)
        .execute(json!({"path": "escape_dir/hijack.txt", "content": "bad"}))
        .await
        .unwrap();
    assert!(write.is_error);
    let out = write.output();
    assert!(
        out.contains("escapes workspace") || out.contains("not allowed"),
        "expected escape/not-allowed error, got: {out}"
    );
    assert!(!outside.join("hijack.txt").exists());
}

#[tokio::test]
async fn a_workspace_descriptor_grants_read_and_write_outside_the_workspace() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("main.rs"), "fn main() {}").unwrap();
    let gate = supervised(home.path());
    let context = Workspace::at(project.path());

    // Without the descriptor the project is unreachable.
    let denied = FileReadTool::new(gate.clone())
        .execute(json!({"path": "main.rs"}))
        .await
        .unwrap();
    assert!(denied.is_error);

    let read = FileReadTool::new(gate.clone())
        .execute_with_context(
            json!({"path": "main.rs"}),
            ToolCallOptions::default(),
            Some(&context),
        )
        .await
        .unwrap();
    assert_eq!(read.output(), "fn main() {}");

    let write = FileWriteTool::new(gate)
        .execute_with_context(
            json!({"path": "new.rs", "content": "// new"}),
            ToolCallOptions::default(),
            Some(&context),
        )
        .await
        .unwrap();
    assert!(!write.is_error, "{}", write.output());
    assert_eq!(
        std::fs::read_to_string(project.path().join("new.rs")).unwrap(),
        "// new"
    );
}

#[tokio::test]
async fn credential_stores_under_a_granted_root_stay_unreachable() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let creds = project.path().join(".ssh");
    std::fs::create_dir(&creds).unwrap();
    std::fs::write(creds.join("id_rsa"), "key").unwrap();
    let context = Workspace::at(project.path());

    let read = FileReadTool::new(supervised(home.path()))
        .execute_with_context(
            json!({"path": ".ssh/id_rsa"}),
            ToolCallOptions::default(),
            Some(&context),
        )
        .await
        .unwrap();
    assert!(
        read.is_error,
        "a granted root must not expose a credential store"
    );
}

#[test]
fn write_tools_prompt_only_when_the_policy_prompts() {
    let dir = tempfile::tempdir().unwrap();
    for (autonomy, prompts) in [
        (AutonomyLevel::Supervised, true),
        (AutonomyLevel::Full, false),
        (AutonomyLevel::ReadOnly, false),
    ] {
        let gate = policy(dir.path(), autonomy, 10);
        assert_eq!(
            EditFileTool::new(gate.clone()).external_effect_with_args(&json!({})),
            prompts
        );
        assert_eq!(
            ApplyPatchTool::new(gate.clone()).external_effect_with_args(&json!({})),
            prompts
        );
        assert_eq!(
            GitOperationsTool::new(gate, PathBuf::from("."))
                .external_effect_with_args(&json!({"operation": "commit"})),
            prompts
        );
    }
}

#[tokio::test]
async fn git_write_operations_are_policy_blocked_in_read_only_mode() {
    let dir = tempfile::tempdir().unwrap();
    let init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(init.success());
    let tool = GitOperationsTool::new(
        policy(dir.path(), AutonomyLevel::ReadOnly, 10),
        dir.path().to_path_buf(),
    );
    let result = tool
        .execute(json!({"operation": "commit", "message": "x"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.output().contains(POLICY_BLOCKED_MARKER),
        "{}",
        result.output()
    );
}

/// The filesystem write gate on the released module, with no gateway stub.
#[cfg(feature = "security-module")]
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn the_write_gate_is_answered_by_the_released_module() {
    use crate::modules::security::released_support::Released;

    let released = Released::new();
    let read_only = released.policy(AutonomyLevel::ReadOnly, 100);
    let supervised = released.policy(AutonomyLevel::Supervised, 100);
    let full = released.policy(AutonomyLevel::Full, 100);

    assert!(!FsGate::write_needs_approval(&read_only));
    assert!(FsGate::write_needs_approval(&supervised));
    assert!(!FsGate::write_needs_approval(&full));
}
