use super::*;
use serde_json::json;
use tempfile::TempDir;

fn make_tool(dir: &TempDir) -> WorkspaceStateTool {
    WorkspaceStateTool::new(dir.path().to_path_buf())
}

#[test]
fn name_is_correct() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(make_tool(&tmp).name(), "read_workspace_state");
}

#[test]
fn description_is_non_empty() {
    let tmp = TempDir::new().unwrap();
    assert!(!make_tool(&tmp).description().is_empty());
}

#[test]
fn schema_is_object_type() {
    let tmp = TempDir::new().unwrap();
    let schema = make_tool(&tmp).parameters_schema();
    assert_eq!(schema["type"], "object");
}

#[test]
fn permission_level_is_read_only() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(
        make_tool(&tmp).permission_level(),
        PermissionLevel::ReadOnly
    );
}

#[tokio::test]
async fn output_contains_git_status_section() {
    let tmp = TempDir::new().unwrap();
    let result = make_tool(&tmp).execute(json!({})).await.unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("Git Status"));
}

#[tokio::test]
async fn include_tree_false_omits_directory_tree() {
    let tmp = TempDir::new().unwrap();
    let result = make_tool(&tmp)
        .execute(json!({"include_tree": false}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(!result.output().contains("Directory Tree"));
}

/// Write a `core.fsmonitor` hook into `dir`'s repository config that
/// creates a marker file when git runs it, and return the marker's path.
///
/// `git status` invokes `core.fsmonitor`, and the value is read from the
/// repository's own `.git/config` — a file inside the workspace, which is
/// where `file_write` puts things. The hook exits non-zero so git falls
/// back to a normal scan and `status` still succeeds; the point of the test
/// is whether the command ran at all, not what it returned.
fn plant_fsmonitor_hook(dir: &TempDir) -> std::path::PathBuf {
    let hook = dir.path().join("hook.sh");
    let marker = dir.path().join("COMMAND_RAN");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\ntouch {:?}\nexit 1\n", marker.to_string_lossy()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // Written with `git config` rather than by appending to the file:
    // appending only lands in `[core]` while `[core]` happens to be the
    // last section, which is true of a fresh `git init` and is not a
    // property worth depending on.
    let ok = std::process::Command::new("git")
        .args(["config", "core.fsmonitor"])
        .arg(&hook)
        .current_dir(dir.path())
        .status()
        .unwrap()
        .success();
    assert!(ok, "failed to plant the hook in the repository config");
    marker
}

fn git_init(dir: &TempDir) {
    // `.status().unwrap()` only unwraps the *spawn*: a non-zero exit would
    // pass silently here and surface later as a confusing assertion about
    // status output. Match `plant_fsmonitor_hook`, which already checks.
    let ok = std::process::Command::new("git")
        .args(["init", "-q", "."])
        .current_dir(dir.path())
        .status()
        .unwrap()
        .success();
    assert!(ok, "git init failed in the test workspace");
}

/// Set a repository config key with `git config`, asserting it took.
fn set_config(dir: &TempDir, key: &str, value: &str) {
    let ok = std::process::Command::new("git")
        .args(["config", key, value])
        .current_dir(dir.path())
        .status()
        .unwrap()
        .success();
    assert!(ok, "failed to set {key} in the test workspace");
}

/// Issue #459. The workspace this tool reads is the same directory the
/// agent's own file tools write to, so its `.git/config` is attacker-
/// controlled input. `core.fsmonitor` names a command that `git status`
/// executes.
///
/// Revert either layer of the fix in `run_git` and this test fails by
/// finding the marker — verified, not assumed.
#[cfg(unix)]
#[tokio::test]
async fn repository_config_naming_a_command_does_not_get_to_run_it() {
    let tmp = TempDir::new().unwrap();
    git_init(&tmp);
    let marker = plant_fsmonitor_hook(&tmp);

    let result = make_tool(&tmp).execute(json!({})).await.unwrap();

    assert!(
        !marker.exists(),
        "`git status` executed the command named by the workspace's own \
         repository config — this tool is a code-execution primitive"
    );
    assert!(
        result.output().contains("fsmonitor"),
        "the refusal should name the key that caused it, got: {}",
        result.output()
    );
}

/// The allowlist has to leave an ordinary repository working, or the fix is
/// just a different way of breaking the tool.
#[cfg(unix)]
#[tokio::test]
async fn an_ordinary_repository_still_reports_status_and_log() {
    let tmp = TempDir::new().unwrap();
    git_init(&tmp);
    std::fs::write(tmp.path().join("tracked.txt"), "hi").unwrap();

    let result = make_tool(&tmp).execute(json!({})).await.unwrap();
    let out = result.output();

    assert!(
        out.contains("tracked.txt"),
        "a plain `git init` workspace must still report status, got: {out}"
    );
    assert!(
        !out.contains("not on the allowlist"),
        "`git init`'s own config must not trip the allowlist, got: {out}"
    );
}

/// The first draft of the allowlist refused any repository carrying an
/// ordinary setting like `core.autocrlf`, which is most of them on Windows
/// and many elsewhere — the tool would have reported nothing useful for a
/// large class of real workspaces. Raised by CodeRabbit on the PR.
#[cfg(unix)]
#[tokio::test]
async fn an_inert_setting_an_ordinary_repository_carries_is_allowed() {
    let tmp = TempDir::new().unwrap();
    git_init(&tmp);
    set_config(&tmp, "core.autocrlf", "input");
    set_config(&tmp, "gc.auto", "0");
    set_config(&tmp, "remote.origin.prune", "true");
    std::fs::write(tmp.path().join("tracked.txt"), "hi").unwrap();

    let out = make_tool(&tmp).execute(json!({})).await.unwrap().output();

    assert!(
        out.contains("tracked.txt"),
        "an ordinary repository must still report status, got: {out}"
    );
    assert!(!out.contains("not on the allowlist"), "got: {out}");
}

/// The other half of the same question, and the answer is the opposite one.
/// `filter.lfs.clean` names a program, so an LFS working copy is refused —
/// fail-closed, and **intended** rather than an oversight. Pinned so that
/// widening the allowlist for ergonomics cannot quietly admit it.
#[cfg(unix)]
#[tokio::test]
async fn an_lfs_clone_is_refused_because_its_filter_names_a_program() {
    let tmp = TempDir::new().unwrap();
    git_init(&tmp);
    // What `git lfs install` writes. `required` is inert and allowed; the
    // three programs are not.
    set_config(&tmp, "filter.lfs.required", "true");
    set_config(&tmp, "filter.lfs.clean", "git-lfs clean -- %f");

    let out = make_tool(&tmp).execute(json!({})).await.unwrap().output();

    assert!(
        out.contains("filter.lfs.clean"),
        "the refusal must name the key that caused it, got: {out}"
    );
}

/// `credential.helper` reads like a preference and is command-valued: a
/// value beginning `!` is run as a shell command. CodeRabbit's review
/// listed it among the inert keys to allow; it is not one, and allowlisting
/// it would have reopened the hole this PR closes.
#[cfg(unix)]
#[tokio::test]
async fn credential_helper_is_refused_despite_looking_like_a_preference() {
    let tmp = TempDir::new().unwrap();
    git_init(&tmp);
    set_config(&tmp, "credential.helper", "!echo pwned");

    let out = make_tool(&tmp).execute(json!({})).await.unwrap().output();

    assert!(
        out.contains("credential.helper"),
        "a command-valued key must be refused however inert it reads, got: {out}"
    );
}

#[test]
fn a_subsection_is_elided_so_one_entry_covers_every_remote() {
    assert_eq!(normalise_config_key("remote.origin.url"), "remote.url");
    assert_eq!(normalise_config_key("remote.a.b.c.url"), "remote.url");
    assert_eq!(normalise_config_key("core.fileMode"), "core.filemode");
    assert_eq!(normalise_config_key("core.fsmonitor"), "core.fsmonitor");
}

#[tokio::test]
async fn lists_non_hidden_files_in_tree() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("readme.txt"), "hi").unwrap();
    std::fs::write(tmp.path().join(".hidden"), "skip").unwrap();
    let result = make_tool(&tmp)
        .execute(json!({"include_tree": true, "recent_commits": 0}))
        .await
        .unwrap();
    assert!(!result.is_error);
    let out = result.output();
    assert!(out.contains("readme.txt"));
    assert!(!out.contains(".hidden"));
}
