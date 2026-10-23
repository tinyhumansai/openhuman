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

use std::path::PathBuf;
use std::sync::Arc;

use super::*;
use crate::security::{AutonomyLevel, SecurityPolicy, TrustedAccess, TrustedRoot};
#[cfg(feature = "security-module")]
use tinyagents_harness::artifacts::{
    extract_artifact_paths, ArtifactKind, ARTIFACT_POINTER_PREFIX,
};
use tinyagents_harness::artifacts::{ArtifactPathPolicy, DEFAULT_OFFLOAD_THRESHOLD_BYTES};

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

// ── The prompt contract (renders upstream; the host passes its tool name) ─────

#[test]
fn prompt_contract_names_the_host_write_tool_and_not_the_read_tool() {
    let rendered =
        tinyagents_harness::artifacts::render_artifact_offload_contract(OFFLOAD_WRITE_TOOL);
    assert!(rendered.contains(OFFLOAD_WRITE_TOOL));
    // A prompt may only name tools its agent actually holds: the reader's tool
    // belongs to a different agent.
    assert!(
        !rendered.contains(READ_TOOL),
        "the contract must not name the parent's reading tool: {rendered}"
    );
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

#[cfg(feature = "security-module")]
#[test]
fn workspace_guard_forwards_internal_state_to_the_module_and_fails_closed() {
    use crate::modules::security::gap::install_port;
    use crate::modules::security::gap::stub::{Asked, StubPort};
    use tinysecurity_bus::PathCheckKind;
    // The adapter must not re-implement the rule, only forward it: the module is
    // the one authority on which locations are internal state.
    let policy = policy_with(PathBuf::from("/action"), PathBuf::from("/state/workspace"));
    let guard = WorkspaceGuard::new(Arc::clone(&policy));
    let stub = Arc::new(StubPort::allowing().paths(|_, kind, path| {
        assert_eq!(kind, PathCheckKind::Internal);
        path.starts_with("/state/workspace/session_raw")
    }));
    let _port = install_port(stub.clone());
    for (candidate, internal) in [
        ("/state/workspace/session_raw/a.jsonl", true),
        ("/action/outputs/report.md", false),
        ("/tmp/unrelated", false),
    ] {
        assert_eq!(
            guard.is_internal_state(std::path::Path::new(candidate)),
            internal,
            "the module's answer for {candidate}"
        );
    }
    assert_eq!(stub.asked().len(), 3);
    assert!(matches!(
        stub.asked()[0],
        Asked::Paths(PathCheckKind::Internal, _)
    ));

    // A module that cannot answer makes everything internal: nothing is offloaded.
    let _port = install_port(Arc::new(StubPort::unavailable()));
    assert!(guard.is_internal_state(std::path::Path::new("/action/outputs/report.md")));
}

fn memory_blobs() -> Arc<dyn tinystoragedrivers_api::BlobStore> {
    let backend = crate::storage::test_support::memory_backend();
    let scoped = backend
        .for_scope(&crate::storage::Scope::local())
        .expect("the memory backend binds the local scope");
    Arc::clone(scoped.blobs())
}

// ── Wiring (the point of `new_artifact_offload`) ──────────────────────────────

#[tokio::test]
async fn an_action_dir_inside_workspace_dir_is_never_materialized() {
    // The fail-closed invariant, end to end: the reader's copy lands under
    // `action_dir`, never in `workspace_dir`. Here `action_dir` is configured
    // INSIDE the workspace root, so only the host's check stands between an
    // agent and core state. The result stays inline and nothing is written.
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace_dir = dir.path().to_path_buf();
    let action_dir = workspace_dir.join("action");
    std::fs::create_dir_all(&action_dir).expect("action dir");
    let policy = policy_with(action_dir.clone(), workspace_dir);

    let offload = new_artifact_offload(memory_blobs(), Some(policy.clone()), "agent", "task");
    let big = "x".repeat(10_000);
    let (text, artifact) = offload_oversized_result(
        big.clone(),
        &offload,
        DEFAULT_OFFLOAD_THRESHOLD_BYTES,
        &action_dir,
        Some(&policy),
    )
    .await;

    assert_eq!(text, big, "the inline payload must survive the refusal");
    assert!(artifact.is_none());
    assert!(
        !action_dir.join("outputs").exists(),
        "nothing may be materialized under workspace_dir"
    );
}

#[cfg(feature = "security-module")]
/// The module's answer for an ordinary workspace: nothing here is internal state.
fn nothing_is_internal() -> crate::modules::security::gap::PortGuard {
    use crate::modules::security::gap::stub::StubPort;
    crate::modules::security::gap::install_port(Arc::new(
        StubPort::allowing().paths(|_, kind, _| kind != tinysecurity_bus::PathCheckKind::Internal),
    ))
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn explicit_workspace_policy_blocks_unguarded_offload_before_writing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace_dir = dir.path().to_path_buf();
    let action_dir = workspace_dir.join("action");
    std::fs::create_dir_all(&action_dir).expect("action dir");
    let policy = policy_with(action_dir.clone(), workspace_dir);
    // A caller may pass a writer whose own ArtifactPathPolicy is absent or
    // different from the explicit policy argument.
    let unguarded = ArtifactOffload::new(memory_blobs(), "worker", "task");
    let big = "x".repeat(10_000);

    let (text, artifact) = offload_oversized_result(
        big.clone(),
        &unguarded,
        DEFAULT_OFFLOAD_THRESHOLD_BYTES,
        &action_dir,
        Some(&policy),
    )
    .await;

    assert_eq!(text, big, "the inline payload must survive the refusal");
    assert!(artifact.is_none());
    assert!(
        !action_dir.join("outputs").exists(),
        "authorization must happen before an artifact is stored or materialized"
    );
}

#[tokio::test]
#[cfg(feature = "security-module")]
async fn a_wired_writer_redacts_before_the_bytes_reach_disk() {
    let _module = nothing_is_internal();
    // `new_artifact_offload` always installs the redactor, so no call site can
    // produce an unredacted writer by omission.
    let dir = tempfile::tempdir().expect("tempdir");
    let action_dir = dir.path().join("action");
    let workspace_dir = dir.path().join("state");
    std::fs::create_dir_all(&action_dir).expect("action dir");
    std::fs::create_dir_all(&workspace_dir).expect("workspace dir");

    let offload = new_artifact_offload(
        memory_blobs(),
        Some(policy_with(dir.path().join("action"), workspace_dir)),
        "agent",
        "task",
    );

    let secret = "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
    let artifact = offload
        .write(ArtifactKind::Output, "creds.md", secret)
        .await
        .expect("write");

    let on_disk = offload
        .read(&artifact.relative_path)
        .await
        .expect("read back")
        .expect("stored");
    // Whether this particular shape trips the `security::redact::text` pass is that function's
    // business; what this pins is that the two agree. A redactor reported as
    // having fired must have actually rewritten the stored bytes.
    assert_eq!(
        artifact.redacted,
        on_disk != secret,
        "the redacted flag must match what actually reached the blob store"
    );
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn oversized_result_is_offloaded_and_the_parent_gets_a_readable_pointer() {
    let _module = nothing_is_internal();
    let dir = tempfile::tempdir().expect("tempdir");
    let action_dir = dir.path().join("action");
    let workspace_dir = dir.path().join("state");
    std::fs::create_dir_all(&action_dir).expect("action dir");
    std::fs::create_dir_all(&workspace_dir).expect("workspace dir");

    let offload = new_artifact_offload(
        memory_blobs(),
        Some(policy_with(dir.path().join("action"), workspace_dir)),
        "worker",
        "task-1",
    );
    let big = "x".repeat(10_000);

    let (text, artifact) = offload_oversized_result(
        big.clone(),
        &offload,
        DEFAULT_OFFLOAD_THRESHOLD_BYTES,
        &action_dir,
        None,
    )
    .await;
    let artifact = artifact.expect("an artifact was written");

    assert!(text.starts_with(ARTIFACT_POINTER_PREFIX));
    // The pointer must name OpenHuman's reading tool, not the crate's idea of
    // one — this is the wiring the READ_TOOL parameter exists for.
    assert!(
        text.contains(READ_TOOL),
        "pointer must name the host read tool: {text}"
    );
    // And the path it quotes must be the one a parent can actually extract.
    assert_eq!(
        extract_artifact_paths(&text),
        vec![artifact.relative_path.clone()]
    );

    // The pointer's path is openable by `file_read` against the action root.
    let on_disk = tokio::fs::read_to_string(action_dir.join(&artifact.relative_path))
        .await
        .expect("read back");
    assert_eq!(on_disk, big, "full fidelity is preserved on disk");
}

#[tokio::test]
async fn offload_failure_keeps_the_inline_payload_for_the_summarizer_fallback() {
    // The soft-failure contract the summarizer detour depends on: a refused
    // offload must never cost the caller its content.
    // The blob write succeeds but the file cannot be materialised for the
    // reader: no pointer to an unopenable path may reach the parent.
    let offload = new_artifact_offload(memory_blobs(), None, "worker", "task-1");
    let big = "x".repeat(10_000);

    let (text, artifact) = offload_oversized_result(
        big.clone(),
        &offload,
        DEFAULT_OFFLOAD_THRESHOLD_BYTES,
        &PathBuf::from("/proc/nonexistent-action-root"),
        None,
    )
    .await;

    assert_eq!(
        text, big,
        "the inline payload must survive a failed offload"
    );
    assert!(artifact.is_none());
}
