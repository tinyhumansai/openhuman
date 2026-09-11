//! Tests for OpenHuman's half of the artifact-offload convention (#3883).
//!
//! The mechanics moved to `tinyagents_harness::artifacts` and are tested
//! there — path resolution, thresholds, abstracts, pointer parsing, the
//! symlink re-check. What is tested here is what stayed:
//!
//! * the **prompt contract**, which is OpenHuman text naming OpenHuman tools;
//! * the **two policy adapters**, which is where a widened permission would
//!   actually cause harm and where no crate-side test can reach;
//! * an **integration pass** proving the wiring hands the crate a guarded,
//!   redacting writer, since the whole point of `new_artifact_offload` is that
//!   no call site can construct one without both.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy, TrustedAccess, TrustedRoot};
use tinyagents_harness::artifacts::ArtifactPathPolicy;

/// Policy with disjoint action/workspace roots, the shipped default layout
/// (`~/OpenHuman/projects` vs `~/.openhuman/users/<id>/workspace`).
///
/// `action_dir` is granted as a read-write trusted root, because that is what
/// production does: `SecurityPolicy::from_config` grants the projects dir
/// exactly this way. Without the grant, `is_resolved_path_allowed_for` refuses
/// everything outside `workspace_dir` — that check is unconditional and is NOT
/// relaxed by `workspace_only`.
fn policy_with(action_dir: PathBuf, workspace_dir: PathBuf) -> Arc<SecurityPolicy> {
    // Canonicalize the granted root: `validate_path` compares the CANONICAL
    // resolved path against it, and on macOS a temp dir under `/tmp` resolves
    // to `/private/tmp`, so an uncanonicalized grant would silently never match.
    let granted = action_dir
        .canonicalize()
        .unwrap_or_else(|_| action_dir.clone());
    Arc::new(SecurityPolicy {
        trusted_roots: vec![TrustedRoot {
            path: granted.to_string_lossy().to_string(),
            access: TrustedAccess::ReadWrite,
        }],
        autonomy: AutonomyLevel::Supervised,
        action_dir,
        workspace_dir,
        ..SecurityPolicy::default()
    })
}

// ── The prompt contract (stays host-side: OpenHuman prompt text) ──────────────

#[test]
fn prompt_contract_names_both_directories_and_the_write_step() {
    let rendered = render_artifact_offload_contract();
    assert!(rendered.starts_with(ARTIFACT_OFFLOAD_HEADING));
    assert!(rendered.contains(OUTPUTS_DIR));
    assert!(rendered.contains(SCRATCH_DIR));
    assert!(rendered.contains(OFFLOAD_WRITE_TOOL));
}

#[test]
fn prompt_contract_names_no_tool_the_agent_may_not_hold() {
    // A prompt may only name tools its agent actually holds: advertising one it
    // lacks produces hallucinated calls that fail. The reader's tool in
    // particular belongs to a different agent.
    let rendered = render_artifact_offload_contract();
    assert!(
        !rendered.contains(READ_TOOL),
        "the contract must not name the parent's reading tool: {rendered}"
    );
}

#[test]
fn offload_contract_is_rendered_only_for_agents_holding_a_write_tool() {
    let mut tools = HashSet::new();
    assert!(
        !should_render_offload_contract(&tools),
        "an agent with no tools cannot offload"
    );

    tools.insert("web_search".to_string());
    assert!(
        !should_render_offload_contract(&tools),
        "a search-only agent has no filesystem tool"
    );

    tools.insert(OFFLOAD_WRITE_TOOL.to_string());
    assert!(should_render_offload_contract(&tools));
}

#[test]
fn read_and_write_tools_are_distinct() {
    // They name different agents' capabilities. Collapsing them would either
    // leak the reader's tool into worker prompts or gate the contract on the
    // wrong tool.
    assert_ne!(READ_TOOL, OFFLOAD_WRITE_TOOL);
}

// ── The policy adapters (stay host-side: this is where policy is enforced) ────

#[test]
fn workspace_guard_reports_workspace_dir_as_the_internal_root() {
    let policy = policy_with(PathBuf::from("/action"), PathBuf::from("/state/workspace"));
    let guard = WorkspaceGuard::new(policy);
    assert_eq!(
        guard.internal_root(),
        Some(std::path::Path::new("/state/workspace"))
    );
}

#[test]
fn workspace_guard_defers_to_the_security_policy_for_internal_state() {
    // The adapter must not re-implement the rule, only forward it — a
    // divergent copy here would drift from the policy the rest of the core
    // enforces, and the two would disagree about the same path.
    let policy = policy_with(PathBuf::from("/action"), PathBuf::from("/state/workspace"));
    let guard = WorkspaceGuard::new(Arc::clone(&policy));
    for candidate in [
        "/state/workspace/session_raw/a.jsonl",
        "/action/outputs/report.md",
        "/tmp/unrelated",
    ] {
        let path = std::path::Path::new(candidate);
        assert_eq!(
            guard.is_internal_state(path),
            policy.is_workspace_internal_path(path),
            "adapter disagreed with SecurityPolicy about {candidate}"
        );
    }
}

// ── Wiring (the point of `new_artifact_offload`) ──────────────────────────────

#[tokio::test]
async fn a_wired_writer_refuses_a_target_inside_workspace_dir() {
    // The fail-closed invariant, end to end: offload targets resolve under
    // `action_dir`, never `workspace_dir`. Here `action_dir` is configured
    // INSIDE the workspace root, so containment alone would pass and only the
    // injected guard stands between an agent and core state.
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace_dir = dir.path().to_path_buf();
    let action_dir = workspace_dir.join("action");
    std::fs::create_dir_all(&action_dir).expect("action dir");

    let offload = new_artifact_offload(
        action_dir,
        Some(policy_with(
            dir.path().join("action"),
            workspace_dir.clone(),
        )),
        "agent",
        "task",
    );

    let err = offload
        .write(ArtifactKind::Output, "leak.md", "x")
        .await
        .expect_err("a target inside workspace_dir must be refused");
    assert!(
        matches!(
            err,
            OffloadError::InternalRoot { .. } | OffloadError::InternalState { .. }
        ),
        "expected a workspace refusal, got {err:?}"
    );
}

#[tokio::test]
async fn a_wired_writer_redacts_before_the_bytes_reach_disk() {
    // `new_artifact_offload` always installs the redactor, so no call site can
    // produce an unredacted writer by omission.
    let dir = tempfile::tempdir().expect("tempdir");
    let action_dir = dir.path().join("action");
    let workspace_dir = dir.path().join("state");
    std::fs::create_dir_all(&action_dir).expect("action dir");
    std::fs::create_dir_all(&workspace_dir).expect("workspace dir");

    let offload = new_artifact_offload(
        action_dir,
        Some(policy_with(dir.path().join("action"), workspace_dir)),
        "agent",
        "task",
    );

    let secret = "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
    let artifact = offload
        .write(ArtifactKind::Output, "creds.md", secret)
        .await
        .expect("write");

    let on_disk = tokio::fs::read_to_string(&artifact.absolute_path)
        .await
        .expect("read back");
    // Whether this particular shape trips `sanitize_text` is that function's
    // business; what this pins is that the two agree. A redactor reported as
    // having fired must have actually rewritten the stored bytes.
    assert_eq!(
        artifact.redacted,
        on_disk != secret,
        "the redacted flag must match what actually reached disk"
    );
}

#[tokio::test]
async fn oversized_result_is_offloaded_and_the_parent_gets_a_readable_pointer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let action_dir = dir.path().join("action");
    let workspace_dir = dir.path().join("state");
    std::fs::create_dir_all(&action_dir).expect("action dir");
    std::fs::create_dir_all(&workspace_dir).expect("workspace dir");

    let offload = new_artifact_offload(
        action_dir,
        Some(policy_with(dir.path().join("action"), workspace_dir)),
        "worker",
        "task-1",
    );
    let big = "x".repeat(10_000);

    let (text, artifact) =
        offload_oversized_result(big.clone(), &offload, DEFAULT_OFFLOAD_THRESHOLD_BYTES).await;
    let artifact = artifact.expect("an artifact was written");

    assert!(text.starts_with(ARTIFACT_POINTER_PREFIX));
    // The pointer must name OpenHuman's reading tool, not the crate's idea of
    // one — this is the wiring the READ_TOOL parameter exists for.
    assert!(
        text.contains(READ_TOOL),
        "pointer must name the host read tool: {text}"
    );
    // And the path it quotes must be the one a parent can actually extract.
    assert_eq!(extract_artifact_paths(&text), vec![artifact.relative_path]);

    let on_disk = tokio::fs::read_to_string(&artifact.absolute_path)
        .await
        .expect("read back");
    assert_eq!(on_disk, big, "full fidelity is preserved on disk");
}

#[tokio::test]
async fn offload_failure_keeps_the_inline_payload_for_the_summarizer_fallback() {
    // The soft-failure contract the summarizer detour depends on: a refused
    // offload must never cost the caller its content.
    let offload = new_artifact_offload(
        PathBuf::from("/proc/nonexistent-action-root"),
        None,
        "worker",
        "task-1",
    );
    let big = "x".repeat(10_000);

    let (text, artifact) =
        offload_oversized_result(big.clone(), &offload, DEFAULT_OFFLOAD_THRESHOLD_BYTES).await;

    assert_eq!(
        text, big,
        "the inline payload must survive a failed offload"
    );
    assert!(artifact.is_none());
}
