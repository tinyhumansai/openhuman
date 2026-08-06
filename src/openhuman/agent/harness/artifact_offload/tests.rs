//! Tests for the artifact-offload convention (#3883).
//!
//! Covers the happy path (oversized result lands in `outputs/`, parent gets a
//! path + abstract), the fallback path (offload refused, inline payload
//! survives for the summarizer/truncation backstop), and the fail-closed path
//! hardening that keeps offload inside `action_dir` and out of `workspace_dir`.

use std::path::PathBuf;
use std::sync::Arc;

use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy, TrustedAccess, TrustedRoot};
use crate::openhuman::tools::traits::Tool;
use crate::openhuman::tools::FileReadTool;
use serde_json::json;

/// Policy with disjoint action/workspace roots, the shipped default layout
/// (`~/OpenHuman/projects` vs `~/.openhuman/users/<id>/workspace`).
///
/// `action_dir` is granted as a read-write trusted root, because that is what
/// production does: `SecurityPolicy::from_config` grants the projects dir
/// exactly this way (`from_config_grants_default_projects_dir_as_readwrite_root`).
/// Without the grant, `is_resolved_path_allowed_for` refuses everything outside
/// `workspace_dir` — that check is unconditional and is NOT relaxed by
/// `workspace_only`, so a hand-built policy that omits the grant cannot read a
/// file the shipped app reads fine.
fn policy_with(action_dir: PathBuf, workspace_dir: PathBuf) -> Arc<SecurityPolicy> {
    // Canonicalize the granted root: `validate_path` compares the CANONICAL
    // resolved path against it, and on macOS a temp dir under `/tmp` resolves
    // to `/private/tmp`, so an uncanonicalized grant would silently never match.
    let granted = action_dir
        .canonicalize()
        .unwrap_or_else(|_| action_dir.clone());
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        trusted_roots: vec![TrustedRoot {
            path: granted.to_string_lossy().to_string(),
            access: TrustedAccess::ReadWrite,
        }],
        action_dir,
        workspace_dir,
        ..SecurityPolicy::default()
    })
}

fn offload_for(action_dir: &std::path::Path, workspace_dir: &std::path::Path) -> ArtifactOffload {
    ArtifactOffload::new(
        action_dir.to_path_buf(),
        Some(policy_with(
            action_dir.to_path_buf(),
            workspace_dir.to_path_buf(),
        )),
        "researcher",
        "sub-1234",
    )
}

// ── Convention directories ──────────────────────────────────────────────────

#[test]
fn kinds_map_to_the_documented_directories() {
    assert_eq!(ArtifactKind::Output.subdir(), OUTPUTS_DIR);
    assert_eq!(ArtifactKind::Output.subdir(), "outputs");
    assert_eq!(ArtifactKind::Scratch.subdir(), SCRATCH_DIR);
    assert_eq!(ArtifactKind::Scratch.subdir(), "workspace");
    assert_eq!(ArtifactKind::Output.as_str(), "output");
    assert_eq!(ArtifactKind::Scratch.as_str(), "scratch");
}

#[test]
fn prompt_contract_names_both_directories_and_the_write_step() {
    let rendered = render_artifact_offload_contract();
    assert!(rendered.starts_with(ARTIFACT_OFFLOAD_HEADING));
    assert!(rendered.contains("`outputs/`"));
    assert!(rendered.contains("`workspace/`"));
    assert!(rendered.contains(OFFLOAD_WRITE_TOOL));
    // Byte-stable: the sub-agent system prompt is prefix-cached.
    assert_eq!(rendered, render_artifact_offload_contract());
}

#[test]
fn prompt_contract_names_no_tool_the_agent_may_not_hold() {
    // A sub-agent prompt may only advertise tools it can actually call, or the
    // model emits calls that fail. `file_write` is gated on the agent holding
    // it; the parent's *reading* tool must never appear at all, since this
    // prompt also reaches agents that have no filesystem tools.
    let rendered = render_artifact_offload_contract();
    assert!(
        !rendered.contains("file_read"),
        "the parent's read tool must not leak into a child prompt: {rendered}"
    );
    // Guarded upstream by researcher::prompt::tests::build_returns_nonempty_body
    // and ops_tests::typed_mode_filters_tools_by_skill_filter.
}

#[test]
fn offload_contract_is_rendered_only_for_agents_holding_a_write_tool() {
    let mut writer = std::collections::HashSet::new();
    writer.insert(OFFLOAD_WRITE_TOOL.to_string());
    assert!(should_render_offload_contract(&writer));

    // `researcher` (search + fetch) and skill-filtered specialists land here.
    let mut reader_only = std::collections::HashSet::new();
    reader_only.insert("web_search_tool".to_string());
    reader_only.insert("notion__search".to_string());
    assert!(!should_render_offload_contract(&reader_only));
    assert!(!should_render_offload_contract(
        &std::collections::HashSet::new()
    ));
}

// ── Path hardening ──────────────────────────────────────────────────────────

#[test]
fn resolves_under_the_convention_directory() {
    let action = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let policy = policy_with(action.path().to_path_buf(), workspace.path().to_path_buf());

    let resolved = resolve_artifact_path(
        action.path(),
        Some(&*policy),
        ArtifactKind::Output,
        "researcher/report.md",
    )
    .expect("a plain relative name resolves");

    assert_eq!(
        resolved,
        action
            .path()
            .join("outputs")
            .join("researcher")
            .join("report.md")
    );
    assert_eq!(
        relative_to_action_dir(action.path(), &resolved),
        "outputs/researcher/report.md"
    );
}

#[test]
fn scratch_kind_resolves_under_action_dir_workspace_not_the_core_workspace() {
    let action = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let policy = policy_with(action.path().to_path_buf(), workspace.path().to_path_buf());

    let resolved = resolve_artifact_path(
        action.path(),
        Some(&*policy),
        ArtifactKind::Scratch,
        "notes.txt",
    )
    .expect("scratch resolves");

    assert!(resolved.starts_with(action.path().join("workspace")));
    assert!(
        !resolved.starts_with(workspace.path()),
        "action_dir/workspace must never be the core workspace_dir"
    );
}

#[test]
fn rejects_parent_traversal() {
    let action = tempfile::tempdir().unwrap();
    let err = resolve_artifact_path(
        action.path(),
        None,
        ArtifactKind::Output,
        "../../etc/passwd",
    )
    .expect_err("`..` traversal must be refused");
    assert!(matches!(err, OffloadError::PathEscape { .. }), "{err}");
}

#[test]
fn rejects_absolute_paths() {
    let action = tempfile::tempdir().unwrap();
    let absolute = if cfg!(windows) {
        "C:\\Windows\\System32\\drivers\\etc\\hosts"
    } else {
        "/etc/passwd"
    };
    let err = resolve_artifact_path(action.path(), None, ArtifactKind::Output, absolute)
        .expect_err("absolute paths must be refused");
    assert!(matches!(err, OffloadError::AbsolutePath { .. }), "{err}");
}

#[test]
fn rejects_empty_and_whitespace_names() {
    let action = tempfile::tempdir().unwrap();
    for name in ["", "   ", "\n\t"] {
        let err = resolve_artifact_path(action.path(), None, ArtifactKind::Output, name)
            .expect_err("empty names must be refused");
        assert!(matches!(err, OffloadError::EmptyName), "{err}");
    }
}

#[test]
fn accepts_leading_current_dir_segments() {
    let action = tempfile::tempdir().unwrap();
    let resolved =
        resolve_artifact_path(action.path(), None, ArtifactKind::Output, "./report.md").unwrap();
    assert_eq!(resolved, action.path().join("outputs").join("report.md"));
}

#[test]
fn rejects_targets_inside_workspace_dir_fail_closed() {
    // action_dir configured INSIDE workspace_dir: every offload target lands in
    // the core's internal state root, so every offload must be refused rather
    // than quietly writing there.
    let workspace = tempfile::tempdir().unwrap();
    let action = workspace.path().join("projects");
    let policy = policy_with(action.clone(), workspace.path().to_path_buf());

    let err = resolve_artifact_path(&action, Some(&*policy), ArtifactKind::Output, "report.md")
        .expect_err("a target under workspace_dir must be refused");
    assert!(matches!(err, OffloadError::WorkspaceTarget { .. }), "{err}");
}

#[test]
fn rejects_workspace_internal_state_paths() {
    // `memory/` is one of the internal state dirs `is_workspace_internal_path`
    // fences off. Pointing action_dir at workspace_dir itself makes
    // `outputs/..`-free names still land on internal state once the caller
    // names one, so the more specific error wins.
    let workspace = tempfile::tempdir().unwrap();
    let action = workspace.path().to_path_buf();
    let policy = policy_with(action.clone(), workspace.path().to_path_buf());

    let err = resolve_artifact_path(&action, Some(&*policy), ArtifactKind::Output, "report.md")
        .expect_err("workspace-rooted action_dir must be refused");
    // Either fail-closed variant is acceptable; both refuse the write.
    assert!(
        matches!(
            err,
            OffloadError::WorkspaceTarget { .. } | OffloadError::WorkspaceInternal { .. }
        ),
        "{err}"
    );
}

#[test]
fn workspace_internal_dir_is_refused_by_the_policy_check() {
    let workspace = tempfile::tempdir().unwrap();
    let policy = policy_with(
        workspace.path().to_path_buf(),
        workspace.path().to_path_buf(),
    );
    // `<workspace>/memory` is workspace-internal; resolve with a kind whose
    // subdir IS `memory` is impossible, so assert the policy predicate the
    // resolver delegates to directly on the path it would build.
    assert!(policy.is_workspace_internal_path(&workspace.path().join("memory")));
    assert!(!policy.is_workspace_internal_path(&workspace.path().join("outputs")));
}

#[test]
fn sanitize_component_strips_separators_and_never_returns_empty() {
    assert_eq!(sanitize_component("researcher"), "researcher");
    assert_eq!(sanitize_component("sub-12/34"), "sub-12_34");
    assert_eq!(sanitize_component("../../etc"), "______etc");
    assert_eq!(sanitize_component(""), "unknown");
    assert_eq!(sanitize_component("///"), "___");
    assert!(sanitize_component(&"x".repeat(500)).chars().count() <= 80);
}

#[test]
fn relative_to_action_dir_falls_back_to_display_for_outside_paths() {
    let action = PathBuf::from("/tmp/action");
    let outside = PathBuf::from("/var/other/file.md");
    assert_eq!(
        relative_to_action_dir(&action, &outside),
        outside.to_string_lossy()
    );
}

// ── Threshold + abstract ────────────────────────────────────────────────────

#[test]
fn should_offload_respects_threshold_and_the_zero_disable() {
    assert!(should_offload(100, 50));
    assert!(!should_offload(50, 50), "exactly at threshold stays inline");
    assert!(!should_offload(10, 50));
    assert!(
        !should_offload(usize::MAX, 0),
        "zero threshold disables offload"
    );
}

#[test]
fn build_abstract_returns_short_content_unchanged() {
    assert_eq!(build_abstract("  short answer  ", 100), "short answer");
}

#[test]
fn build_abstract_cuts_at_a_line_boundary_when_one_is_available() {
    let content = format!("{}\n{}", "a".repeat(60), "b".repeat(200));
    let out = build_abstract(&content, 100);
    assert!(out.ends_with("..."));
    assert!(!out.contains('b'), "should stop at the line break: {out}");
}

#[test]
fn build_abstract_cuts_at_a_word_boundary_when_there_is_no_line_break() {
    let content = format!("{} {}", "word ".repeat(20), "tail".repeat(50));
    let out = build_abstract(&content, 60);
    assert!(out.ends_with("..."));
    assert!(out.chars().count() <= 64);
}

#[test]
fn build_abstract_handles_a_zero_budget_and_boundary_free_text() {
    assert_eq!(build_abstract("anything", 0), "");
    let out = build_abstract(&"x".repeat(500), 40);
    assert!(out.ends_with("..."));
    assert!(out.chars().count() <= 44);
}

#[test]
fn build_abstract_never_splits_a_multibyte_character() {
    let content = "é".repeat(400);
    let out = build_abstract(&content, 50);
    assert!(out.ends_with("..."));
    assert!(out.chars().all(|c| c == 'é' || c == '.'));
}

// ── Pointer render + parse ──────────────────────────────────────────────────

fn sample_artifact(redacted: bool) -> OffloadedArtifact {
    OffloadedArtifact {
        kind: ArtifactKind::Output,
        relative_path: "outputs/researcher/sub-1234-result.md".to_string(),
        absolute_path: PathBuf::from("/tmp/action/outputs/researcher/sub-1234-result.md"),
        stored_bytes: 4096,
        original_bytes: 4096,
        redacted,
    }
}

#[test]
fn pointer_carries_path_size_and_a_file_read_call() {
    let rendered = render_artifact_pointer(&sample_artifact(false), "two-line abstract");
    assert!(rendered.starts_with(ARTIFACT_POINTER_PREFIX));
    assert!(rendered.contains("kind=output"));
    assert!(rendered.contains("path=outputs/researcher/sub-1234-result.md"));
    assert!(rendered.contains("bytes=4096"));
    assert!(rendered
        .contains(r#"read_with: file_read {"path":"outputs/researcher/sub-1234-result.md"}"#));
    assert!(rendered.contains("[abstract]\ntwo-line abstract"));
    assert!(!rendered.contains("redaction was applied"));
}

#[test]
fn pointer_discloses_redaction_when_it_happened() {
    let rendered = render_artifact_pointer(&sample_artifact(true), "abstract");
    assert!(rendered.contains("Credential/PII redaction was applied"));
}

#[test]
fn extract_artifact_paths_reads_pointers_out_of_a_handoff() {
    let rendered = render_artifact_pointer(&sample_artifact(false), "abstract");
    assert_eq!(
        extract_artifact_paths(&rendered),
        vec!["outputs/researcher/sub-1234-result.md".to_string()]
    );
}

#[test]
fn extract_artifact_paths_dedupes_and_keeps_encounter_order() {
    let text = "[artifact] kind=output path=outputs/a.md bytes=1\n\
                prose in between\n\
                  [artifact] kind=scratch path=workspace/b.md bytes=2\n\
                [artifact] kind=output path=outputs/a.md bytes=1\n";
    assert_eq!(
        extract_artifact_paths(text),
        vec!["outputs/a.md".to_string(), "workspace/b.md".to_string()]
    );
}

#[test]
fn extract_artifact_paths_ignores_non_pointer_and_malformed_lines() {
    let text = "ordinary answer text\n\
                [artifact] kind=output bytes=1\n\
                [artifact] kind=output path= bytes=1\n\
                the word [artifact] appearing mid-sentence path=nope\n";
    assert!(extract_artifact_paths(text).is_empty());
    assert!(extract_artifact_paths("").is_empty());
}

#[test]
fn note_artifact_handoff_reports_how_many_paths_crossed() {
    let paths = vec!["outputs/a.md".to_string(), "outputs/b.md".to_string()];
    assert_eq!(
        note_artifact_handoff(HANDOFF_STAGE_RECORDED, "researcher", "sub-1", &paths),
        2
    );
    assert_eq!(
        note_artifact_handoff(HANDOFF_STAGE_CONSUMED, "researcher", "sub-1", &[]),
        0
    );
    // The two ends of one pointer must be distinguishable in a run journal.
    assert_ne!(HANDOFF_STAGE_RECORDED, HANDOFF_STAGE_CONSUMED);
}

#[test]
fn offload_threshold_tightens_to_an_agents_own_result_cap() {
    // A cap below the default would truncate the result before offload ever
    // fired (flow_memory_agent 4 000, context_scout 5 000).
    assert_eq!(
        effective_offload_threshold(DEFAULT_OFFLOAD_THRESHOLD_BYTES, Some(4_000)),
        4_000
    );
    // A cap above the default leaves the default in charge.
    assert_eq!(
        effective_offload_threshold(DEFAULT_OFFLOAD_THRESHOLD_BYTES, Some(50_000)),
        DEFAULT_OFFLOAD_THRESHOLD_BYTES
    );
    // Uncapped agents (agent_memory) and a nonsense zero cap keep the default.
    assert_eq!(
        effective_offload_threshold(DEFAULT_OFFLOAD_THRESHOLD_BYTES, None),
        DEFAULT_OFFLOAD_THRESHOLD_BYTES
    );
    assert_eq!(
        effective_offload_threshold(DEFAULT_OFFLOAD_THRESHOLD_BYTES, Some(0)),
        DEFAULT_OFFLOAD_THRESHOLD_BYTES
    );
}

// ── Write path ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn write_persists_under_outputs_and_reports_the_relative_path() {
    let action = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let offload = offload_for(action.path(), workspace.path());

    let artifact = offload
        .write(ArtifactKind::Output, "researcher/report.md", "full body")
        .await
        .expect("write succeeds");

    assert_eq!(artifact.relative_path, "outputs/researcher/report.md");
    assert_eq!(artifact.kind, ArtifactKind::Output);
    assert!(!artifact.redacted);
    assert_eq!(
        tokio::fs::read_to_string(&artifact.absolute_path)
            .await
            .unwrap(),
        "full body"
    );
}

#[tokio::test]
async fn write_redacts_credentials_before_they_reach_disk() {
    let action = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let offload = offload_for(action.path(), workspace.path());

    let body = "findings ghp_abcdefghijklmnopqrstuvwxyz123456";
    let artifact = offload
        .write(ArtifactKind::Output, "leaky.md", body)
        .await
        .unwrap();

    assert!(artifact.redacted, "the token must be scrubbed");
    let stored = tokio::fs::read_to_string(&artifact.absolute_path)
        .await
        .unwrap();
    assert!(!stored.contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
}

#[tokio::test]
async fn write_refuses_a_traversal_target_without_touching_disk() {
    let action = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let offload = offload_for(action.path(), workspace.path());

    let err = offload
        .write(ArtifactKind::Output, "../escaped.md", "body")
        .await
        .expect_err("traversal must be refused");

    assert!(matches!(err, OffloadError::PathEscape { .. }), "{err}");
    assert!(!action.path().join("escaped.md").exists());
}

#[tokio::test]
async fn default_result_name_sanitizes_both_identifiers() {
    let action = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let offload = ArtifactOffload::new(
        action.path().to_path_buf(),
        Some(policy_with(
            action.path().to_path_buf(),
            workspace.path().to_path_buf(),
        )),
        "team/researcher",
        "sub/../1",
    );

    assert_eq!(
        offload.default_result_name(),
        "team_researcher/sub____1-result.md"
    );
    assert_eq!(offload.action_dir(), action.path());
    assert!(offload
        .resolve(ArtifactKind::Output, &offload.default_result_name())
        .is_ok());
}

// ── End-to-end offload ──────────────────────────────────────────────────────

#[tokio::test]
async fn oversized_result_is_offloaded_and_the_parent_gets_a_path_plus_abstract() {
    let action = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let offload = offload_for(action.path(), workspace.path());

    let body = format!("HEADLINE FINDING\n{}", "detail line\n".repeat(2_000));
    let (handed_to_parent, artifact) =
        offload_oversized_result(body.clone(), &offload, DEFAULT_OFFLOAD_THRESHOLD_BYTES).await;

    let artifact = artifact.expect("an oversized result must be offloaded");
    assert_eq!(
        artifact.relative_path,
        "outputs/researcher/sub-1234-result.md"
    );
    assert!(
        handed_to_parent.len() < body.len(),
        "the pointer must be smaller than the payload it replaces"
    );
    assert!(handed_to_parent.starts_with(ARTIFACT_POINTER_PREFIX));
    assert!(
        handed_to_parent.contains("HEADLINE FINDING"),
        "abstract keeps the lede"
    );
    assert_eq!(
        extract_artifact_paths(&handed_to_parent),
        vec![artifact.relative_path.clone()]
    );

    // Full fidelity survives on disk — the whole point of the convention. The
    // body is byte-identical to what the worker produced, not an abstract.
    assert_eq!(
        tokio::fs::read_to_string(&artifact.absolute_path)
            .await
            .unwrap(),
        body
    );

    // And the parent recovers it with an ordinary relative `file_read`, which
    // resolves against action_dir under the same trusted-root grant production
    // gives the projects dir.
    let reader_policy = policy_with(action.path().to_path_buf(), workspace.path().to_path_buf());
    let read = FileReadTool::new(reader_policy)
        .execute(json!({ "path": artifact.relative_path }))
        .await
        .unwrap();
    assert!(!read.is_error, "{}", read.output());
    assert!(read.output().contains("HEADLINE FINDING"));
}

#[tokio::test]
async fn small_result_stays_inline() {
    let action = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let offload = offload_for(action.path(), workspace.path());

    let (out, artifact) = offload_oversized_result(
        "short answer".to_string(),
        &offload,
        DEFAULT_OFFLOAD_THRESHOLD_BYTES,
    )
    .await;

    assert_eq!(out, "short answer");
    assert!(artifact.is_none());
    assert!(!action.path().join("outputs").exists(), "no file written");
}

#[tokio::test]
async fn offload_failure_keeps_the_inline_payload_for_the_summarizer_fallback() {
    // action_dir inside workspace_dir: every target is refused fail-closed, so
    // the caller must get its payload back untouched rather than losing it.
    let workspace = tempfile::tempdir().unwrap();
    let action = workspace.path().join("projects");
    let offload = ArtifactOffload::new(
        action.clone(),
        Some(policy_with(action, workspace.path().to_path_buf())),
        "researcher",
        "sub-1234",
    );

    let body = "y".repeat(DEFAULT_OFFLOAD_THRESHOLD_BYTES + 1);
    let (out, artifact) =
        offload_oversized_result(body.clone(), &offload, DEFAULT_OFFLOAD_THRESHOLD_BYTES).await;

    assert_eq!(
        out, body,
        "the inline payload must survive a refused offload"
    );
    assert!(artifact.is_none());
}

#[tokio::test]
async fn abstract_is_built_from_the_redacted_body_not_the_raw_output() {
    // The pointer goes straight into the parent's context. Building its
    // abstract from the raw output would re-expose exactly the credential
    // `write` scrubbed out of the file on disk.
    let action = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let offload = offload_for(action.path(), workspace.path());

    let secret = "ghp_abcdefghijklmnopqrstuvwxyz123456";
    let body = format!("{secret}\n{}", "filler line\n".repeat(2_000));
    let (handed_to_parent, artifact) =
        offload_oversized_result(body, &offload, DEFAULT_OFFLOAD_THRESHOLD_BYTES).await;

    let artifact = artifact.expect("offloaded");
    assert!(artifact.redacted);
    assert!(
        !handed_to_parent.contains(secret),
        "the abstract leaked a credential that was redacted on disk: {handed_to_parent}"
    );
    let stored = tokio::fs::read_to_string(&artifact.absolute_path)
        .await
        .unwrap();
    assert!(!stored.contains(secret));
}

// Unix-only: creating a directory symlink on Windows needs a privilege the CI
// runner does not have. The guard itself is platform-independent.
#[cfg(unix)]
#[tokio::test]
async fn write_refuses_a_parent_that_symlinks_out_of_the_convention_root() {
    // `resolve_artifact_path` is lexical by necessity (the target does not
    // exist yet), so a pre-existing symlink is only catchable once the parent
    // materialises. Without this check the write would follow the link.
    let action = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();

    let outputs_root = action.path().join("outputs");
    tokio::fs::create_dir_all(&outputs_root).await.unwrap();
    std::os::unix::fs::symlink(outside.path(), outputs_root.join("linked")).unwrap();

    let offload = offload_for(action.path(), workspace.path());
    let err = offload
        .write(ArtifactKind::Output, "linked/report.md", "body")
        .await
        .expect_err("a symlinked parent must be refused");

    assert!(matches!(err, OffloadError::SymlinkEscape { .. }), "{err}");
    assert!(
        !outside.path().join("report.md").exists(),
        "nothing may be written through the link"
    );
}

#[tokio::test]
async fn worktree_artifact_renders_a_path_the_parent_can_resolve() {
    // The worker writes inside its isolated checkout; the parent resolves
    // relative paths against its own action root. Rendering against the
    // parent's root keeps the pointer meaningful instead of handing back a bare
    // `outputs/…` that would miss the file.
    let parent_action = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let worktree = parent_action.path().join("worktrees").join("w1");

    let nested = ArtifactOffload::new(
        worktree.clone(),
        Some(policy_with(
            parent_action.path().to_path_buf(),
            workspace.path().to_path_buf(),
        )),
        "code_executor",
        "sub-7",
    )
    .with_render_root(parent_action.path().to_path_buf());

    let artifact = nested
        .write(ArtifactKind::Output, "report.md", "body")
        .await
        .unwrap();
    assert_eq!(
        artifact.relative_path, "worktrees/w1/outputs/report.md",
        "a nested worktree stays relative to the parent's action root"
    );

    // A worktree OUTSIDE the parent's root cannot be expressed relatively, so
    // the pointer carries the absolute path rather than a wrong relative one.
    let outside = tempfile::tempdir().unwrap();
    let detached = ArtifactOffload::new(outside.path().to_path_buf(), None, "code_executor", "s8")
        .with_render_root(parent_action.path().to_path_buf());
    let artifact = detached
        .write(ArtifactKind::Output, "report.md", "body")
        .await
        .unwrap();
    assert!(
        std::path::Path::new(&artifact.relative_path).is_absolute(),
        "expected an absolute fallback, got {}",
        artifact.relative_path
    );
}

#[tokio::test]
async fn offload_is_disabled_by_a_zero_threshold() {
    let action = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let offload = offload_for(action.path(), workspace.path());

    let body = "z".repeat(100_000);
    let (out, artifact) = offload_oversized_result(body.clone(), &offload, 0).await;

    assert_eq!(out, body);
    assert!(artifact.is_none());
}

#[tokio::test]
async fn offload_without_a_policy_still_enforces_containment() {
    let action = tempfile::tempdir().unwrap();
    let offload = ArtifactOffload::new(action.path().to_path_buf(), None, "planner", "sub-9");

    let artifact = offload
        .write(ArtifactKind::Scratch, "plan.md", "scratch body")
        .await
        .expect("no policy means no workspace checks, containment still applies");
    assert_eq!(artifact.relative_path, "workspace/plan.md");

    let err = offload
        .write(ArtifactKind::Scratch, "../../outside.md", "body")
        .await
        .expect_err("traversal is refused with or without a policy");
    assert!(matches!(err, OffloadError::PathEscape { .. }), "{err}");
}
