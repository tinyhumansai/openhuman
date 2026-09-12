//! Tests for [`super::security_for_tool_context`] — the per-tool-call policy
//! clone that carries a run's `WorkspaceDescriptor` into the filesystem tools.
//!
//! Split out of `mod.rs` to keep the module root export-focused. Declared via
//! `#[path = "mod_tests.rs"] mod tests;` so `super::*` resolves to the
//! `filesystem` module.

use super::*;
use crate::openhuman::security::policy::TrustedAccess;
use std::path::Path;
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_harness::tool::ToolExecutionContext;
use tinyagents_harness::workspace::WorkspaceDescriptor;

/// A run context carrying a workspace descriptor rooted at `root`, exactly as
/// the session builder / sub-agent runner / `cwd` RPC param produce one.
fn tool_context_with_workspace(root: &Path) -> ToolExecutionContext {
    let ws = WorkspaceDescriptor::new(root.to_path_buf()).with_policy_id("test-descriptor");
    let ctx: RunContext = RunContext::new(RunConfig::new("test-run"), ()).with_workspace(ws);
    ToolExecutionContext::from_run_context(&ctx)
}

/// A policy whose `workspace_dir`/`action_dir` are the OpenHuman home — i.e. a
/// directory that does NOT contain the descriptor root. `workspace_only` is on,
/// matching the default tier, so an absolute path outside the workspace is
/// refused unless something grants it.
fn policy_rooted_at(home: &Path) -> SecurityPolicy {
    SecurityPolicy {
        workspace_dir: home.to_path_buf(),
        action_dir: home.to_path_buf(),
        workspace_only: true,
        ..SecurityPolicy::default()
    }
}

#[tokio::test]
async fn descriptor_root_outside_workspace_is_readable_and_writable() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let file = project.path().join("main.rs");
    std::fs::write(&file, b"fn main() {}").expect("write file");

    let base = policy_rooted_at(home.path());
    // Baseline: without a descriptor the project is unreachable.
    assert!(
        base.validate_path(&file.to_string_lossy()).await.is_err(),
        "a path outside workspace_dir must be refused without a grant"
    );

    let ctx = tool_context_with_workspace(project.path());
    let scoped = security_for_tool_context(&base, Some(&ctx), "file_read");

    scoped
        .validate_path(&file.to_string_lossy())
        .await
        .expect("absolute path under the descriptor root reads");
    scoped
        .validate_parent_path(&project.path().join("new.rs").to_string_lossy())
        .await
        .expect("absolute path under the descriptor root writes");
    // Relative paths resolve against the descriptor root too.
    scoped
        .validate_path("main.rs")
        .await
        .expect("relative path under the descriptor root reads");
    scoped
        .validate_parent_path("sub/new.rs")
        .await
        .expect("relative path under the descriptor root writes");
}

#[tokio::test]
async fn sibling_directory_outside_the_descriptor_root_still_fails() {
    let home = tempfile::tempdir().expect("home");
    let parent = tempfile::tempdir().expect("parent");
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir(&project).expect("project dir");
    std::fs::create_dir(&sibling).expect("sibling dir");
    let secret = sibling.join("secret.txt");
    std::fs::write(&secret, b"nope").expect("write secret");

    let ctx = tool_context_with_workspace(&project);
    let scoped = security_for_tool_context(&policy_rooted_at(home.path()), Some(&ctx), "file_read");

    assert!(
        scoped
            .validate_path(&secret.to_string_lossy())
            .await
            .is_err(),
        "a sibling of the granted root must stay unreachable"
    );
    assert!(
        scoped
            .validate_parent_path(&sibling.join("new.txt").to_string_lossy())
            .await
            .is_err(),
        "writes into a sibling of the granted root must stay refused"
    );
}

#[tokio::test]
async fn always_forbidden_paths_under_a_granted_root_still_fail() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    // `.ssh` is an unconditional credential store: a grant on its parent must
    // never expose it (`SecurityPolicy::is_always_forbidden`).
    let creds = project.path().join(".ssh");
    std::fs::create_dir(&creds).expect("creds dir");
    let key = creds.join("id_rsa");
    std::fs::write(&key, b"key").expect("write key");

    let ctx = tool_context_with_workspace(project.path());
    let scoped = security_for_tool_context(&policy_rooted_at(home.path()), Some(&ctx), "file_read");

    assert!(
        scoped.validate_path(&key.to_string_lossy()).await.is_err(),
        "a granted root must not expose a credential store beneath it"
    );
    assert!(
        scoped.validate_path(".ssh/id_rsa").await.is_err(),
        "the relative form is refused too"
    );
    assert!(
        scoped
            .validate_parent_path(&creds.join("new_key").to_string_lossy())
            .await
            .is_err(),
        "writes into a credential store beneath a granted root stay refused"
    );
}

#[tokio::test]
async fn workspace_internal_state_under_a_granted_root_still_fails() {
    // A descriptor rooted at `workspace_dir` itself must not open the
    // core-managed internal state beneath it.
    let home = tempfile::tempdir().expect("home");
    let internal = home.path().join("sessions");
    std::fs::create_dir(&internal).expect("internal dir");
    let transcript = internal.join("session.json");
    std::fs::write(&transcript, b"{}").expect("write transcript");

    let ctx = tool_context_with_workspace(home.path());
    let scoped = security_for_tool_context(&policy_rooted_at(home.path()), Some(&ctx), "file_read");

    assert!(
        scoped
            .validate_path(&transcript.to_string_lossy())
            .await
            .is_err(),
        "workspace-internal state stays unreachable through a grant"
    );
}

#[test]
fn no_descriptor_leaves_the_policy_untouched() {
    let home = tempfile::tempdir().expect("home");
    let base = policy_rooted_at(home.path());

    let scoped = security_for_tool_context(&base, None, "file_read");
    assert_eq!(scoped.action_dir, base.action_dir);
    assert_eq!(scoped.workspace_dir, base.workspace_dir);
    assert_eq!(scoped.trusted_roots, base.trusted_roots);

    // ... and an all-default context with no workspace behaves the same.
    let ctx: ToolExecutionContext =
        ToolExecutionContext::from_run_context(&RunContext::new(RunConfig::new("test-run"), ()));
    let scoped = security_for_tool_context(&base, Some(&ctx), "file_read");
    assert_eq!(scoped.action_dir, base.action_dir);
    assert_eq!(scoped.trusted_roots, base.trusted_roots);
}

#[test]
fn descriptor_grant_is_read_write_and_additive() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let mut base = policy_rooted_at(home.path());
    base.trusted_roots
        .push(crate::openhuman::security::policy::TrustedRoot {
            path: "/pre/existing".to_string(),
            access: TrustedAccess::Read,
        });

    let ctx = tool_context_with_workspace(project.path());
    let scoped = security_for_tool_context(&base, Some(&ctx), "file_read");

    assert_eq!(
        scoped.trusted_roots.len(),
        base.trusted_roots.len() + 1,
        "the grant is appended, never replacing configured roots"
    );
    let granted = scoped.trusted_roots.last().expect("granted root");
    assert_eq!(granted.path, project.path().to_string_lossy());
    assert_eq!(granted.access, TrustedAccess::ReadWrite);
}
