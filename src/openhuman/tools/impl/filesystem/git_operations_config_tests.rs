//! Repository-config hardening (#5494) — the tests for
//! [`super::super::git_operations_config`].
//!
//! Split out of `git_operations_tests.rs` for the Rust layout gate, along the
//! same seam as the module split: these exercise the allow-list and the
//! hardened `git` invocation rather than the command surface.
//!
//! Attached to `git_operations` (not to the config module) so `use super::*`
//! still resolves `GitOperationsTool` the way it did before the split, and so
//! the fixtures in the sibling test module stay reachable.

use super::super::git_operations_config::normalise_config_key;
// The fixtures stay in `git_operations_tests.rs` and are shared rather than
// duplicated: both modules are children of `git_operations`, so `pub(super)`
// there makes them reachable here.
use super::tests::{error_text, hermetic, init_git_repo, test_tool};
use super::*;
use tempfile::TempDir;

// ── run_git_command_in: repository config hardening (issue #5494) ─────────
//
// `run_git_command_in` backs every operation this tool exposes, including
// `status`, which — like `read_workspace_state`'s `run_git` before #5493 —
// invokes `core.fsmonitor` from the repository's own `.git/config`. That file
// lives in `action_dir`, which `file_write` and `git_operations` itself
// (`add`, `commit`, `checkout`) can write to, so it is attacker-controlled
// input, not trusted configuration.

/// Write a `core.fsmonitor` hook into `dir`'s repository config that creates a
/// marker file when git runs it, and return the marker's path.
///
/// Runs the hook once up front and asserts the marker appears, then removes
/// it — so a later absent marker means the hardening refused the hook, not
/// that the hook itself was silently broken (e.g. by `{:?}`-escaping a path
/// the shell would quote differently than Rust's `Debug` does).
#[cfg(unix)]
fn plant_fsmonitor_hook(dir: &std::path::Path) -> std::path::PathBuf {
    let hook = dir.join("hook.sh");
    let marker = dir.join("COMMAND_RAN");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\ntouch {:?}\nexit 1\n", marker.to_string_lossy()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    std::process::Command::new(&hook).status().unwrap();
    assert!(marker.exists(), "the planted hook does not run at all");
    std::fs::remove_file(&marker).unwrap();

    // Written with `git config` rather than by appending to the file:
    // appending only lands in `[core]` while `[core]` happens to be the last
    // section, which is true of a fresh `git init` and is not a property
    // worth depending on.
    let ok = hermetic(
        std::process::Command::new("git")
            .args(["config", "core.fsmonitor"])
            .arg(&hook)
            .current_dir(dir),
    )
    .status()
    .unwrap()
    .success();
    assert!(ok, "failed to plant the hook in the repository config");
    marker
}

/// Set a repository config key with `git config`, asserting it took.
fn set_config(dir: &std::path::Path, key: &str, value: &str) {
    let ok = hermetic(
        std::process::Command::new("git")
            .args(["config", key, value])
            .current_dir(dir),
    )
    .status()
    .unwrap()
    .success();
    assert!(ok, "failed to set {key} in the test workspace");
}

/// Issue #5494. `git status` executes the command named by the workspace's
/// own repository config unless `run_git_command_in` refuses to run under it.
/// Revert the hardening and this test fails by finding the marker — verified,
/// not assumed.
#[cfg(unix)]
#[tokio::test]
async fn repository_config_naming_a_command_does_not_get_to_run_it() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    let marker = plant_fsmonitor_hook(tmp.path());

    let tool = test_tool(tmp.path());
    let result = tool.execute(json!({"operation": "status"})).await;
    let msg = error_text(&result);

    assert!(
        !marker.exists(),
        "`git status` executed the command named by the workspace's own \
         repository config — this tool is a code-execution primitive"
    );
    assert!(
        msg.contains("fsmonitor"),
        "the refusal should name the key that caused it, got: {msg}"
    );
}

/// The allowlist has to leave an ordinary repository working, or the fix is
/// just a different way of breaking the tool.
#[tokio::test]
async fn an_ordinary_repository_still_reports_status() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    std::fs::write(tmp.path().join("tracked.txt"), "hi").unwrap();

    let tool = test_tool(tmp.path());
    let result = tool.execute(json!({"operation": "status"})).await.unwrap();

    assert!(!result.is_error, "got: {}", result.output());
    assert!(
        result.output().contains("tracked.txt"),
        "a plain `git init` workspace must still report status, got: {}",
        result.output()
    );
}

/// A first-draft allowlist that refused any repository carrying an ordinary
/// setting like `core.autocrlf` would report nothing useful for a large class
/// of real workspaces.
#[tokio::test]
async fn an_inert_setting_an_ordinary_repository_carries_is_allowed() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    set_config(tmp.path(), "core.autocrlf", "input");
    set_config(tmp.path(), "gc.auto", "0");
    set_config(tmp.path(), "remote.origin.prune", "true");

    let tool = test_tool(tmp.path());
    let result = tool.execute(json!({"operation": "status"})).await.unwrap();

    assert!(
        !result.is_error && !result.output().contains("not on the allowlist"),
        "an ordinary repository must still report status, got: {}",
        result.output()
    );
}

/// The other half of the same question, and the answer is the opposite one.
/// `filter.lfs.clean` names a program, so an LFS working copy is refused —
/// fail-closed, and intended rather than an oversight.
#[tokio::test]
async fn an_lfs_clone_is_refused_because_its_filter_names_a_program() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    // What `git lfs install` writes. `required` is inert and allowed; the
    // three programs are not.
    set_config(tmp.path(), "filter.lfs.required", "true");
    set_config(tmp.path(), "filter.lfs.clean", "git-lfs clean -- %f");

    let tool = test_tool(tmp.path());
    let result = tool.execute(json!({"operation": "status"})).await;
    let msg = error_text(&result);

    assert!(
        msg.contains("filter.lfs.clean"),
        "the refusal must name the key that caused it, got: {msg}"
    );
}

/// `credential.helper` reads like a preference and is command-valued: a value
/// beginning `!` is run as a shell command. It must be refused however inert
/// it reads.
#[tokio::test]
async fn credential_helper_is_refused_despite_looking_like_a_preference() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    set_config(tmp.path(), "credential.helper", "!echo pwned");

    let tool = test_tool(tmp.path());
    let result = tool.execute(json!({"operation": "status"})).await;
    let msg = error_text(&result);

    assert!(
        msg.contains("credential.helper"),
        "a command-valued key must be refused however inert it reads, got: {msg}"
    );
}

/// The refusal must hold for a write operation too, not just `status` — the
/// same repository config runs under `commit`/`add`/`checkout`/`stash`.
#[tokio::test]
async fn refusal_also_covers_write_operations() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    set_config(tmp.path(), "credential.helper", "!echo pwned");

    let tool = test_tool(tmp.path());
    let result = tool
        .execute(json!({"operation": "commit", "message": "test"}))
        .await
        .unwrap();

    assert!(
        result.is_error && result.output().contains("credential.helper"),
        "write operations must be refused under untrusted repo config too, got: {}",
        result.output()
    );
}

/// `core.worktree` redirects the working-tree root every write operation
/// here (`checkout`, `add`, `commit`, `stash`) targets. Left on the
/// allowlist, a repository config could point that root outside
/// `action_dir` and turn a supposedly sandboxed write into one against an
/// arbitrary directory. Nothing this tool does needs the key — worktree
/// isolation goes through `WorkspaceDescriptor` instead.
#[tokio::test]
async fn core_worktree_is_refused_because_it_can_redirect_writes_outside_the_sandbox() {
    let tmp = TempDir::new().unwrap();
    let elsewhere = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    set_config(
        tmp.path(),
        "core.worktree",
        &elsewhere.path().to_string_lossy(),
    );

    let tool = test_tool(tmp.path());
    let result = tool.execute(json!({"operation": "status"})).await;
    let msg = error_text(&result);

    assert!(
        msg.contains("core.worktree"),
        "the refusal must name the key that caused it, got: {msg}"
    );
}

/// `extensions.worktreeConfig` is itself allowlisted as an ordinary setting,
/// but turning it on makes git additionally read `config.worktree` — a
/// second file `--local` alone does not see. A `core.hooksPath` set there is
/// invisible to a `--local`-only inspection and would still run on the next
/// `commit`. The inspection step must read the same merged view git does.
#[tokio::test]
async fn a_hookspath_hidden_in_worktree_scoped_config_is_still_refused() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    set_config(tmp.path(), "extensions.worktreeConfig", "true");
    // What `git config --worktree core.hooksPath <dir>` writes; `set_config`
    // only reaches `--local`, so this key is planted directly the same way
    // the production inspection step reads it — via a real `git config
    // --worktree` invocation — to prove the bypass is closed, not just that
    // `set_config` happens to skip it.
    let ok = hermetic(
        std::process::Command::new("git")
            .args(["config", "--worktree", "core.hooksPath"])
            .arg(tmp.path())
            .current_dir(tmp.path()),
    )
    .status()
    .unwrap()
    .success();
    assert!(ok, "failed to set core.hooksPath in worktree-scoped config");

    let tool = test_tool(tmp.path());
    let result = tool.execute(json!({"operation": "status"})).await;
    let msg = error_text(&result);

    assert!(
        msg.contains("core.hookspath"),
        "a hookspath hidden in worktree-scoped config must still be refused, got: {msg}"
    );
}

/// The allowlist inspection and the real command are two separate `git`
/// invocations, so a config change landing in the gap between them would be
/// invisible to the first and still reach the second. This test calls
/// `hardened_git` directly — skipping `first_disallowed_repo_config_key`
/// entirely, standing in for that gap — to prove the second invocation does
/// not depend on the first having caught anything: `core.hooksPath` is
/// neutralised at the point of execution regardless of what any inspection
/// saw or missed.
#[cfg(unix)]
#[tokio::test]
async fn hardened_git_neutralises_hookspath_even_if_the_allowlist_check_never_ran() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    set_config(tmp.path(), "user.email", "test@example.com");
    set_config(tmp.path(), "user.name", "Test");

    let hooks_dir = tmp.path().join("evil-hooks");
    std::fs::create_dir(&hooks_dir).unwrap();
    let marker = tmp.path().join("HOOK_RAN");
    std::fs::write(
        hooks_dir.join("pre-commit"),
        format!("#!/bin/sh\ntouch {:?}\nexit 0\n", marker.to_string_lossy()),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            hooks_dir.join("pre-commit"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    set_config(tmp.path(), "core.hooksPath", &hooks_dir.to_string_lossy());

    std::fs::write(tmp.path().join("f.txt"), "hi").unwrap();
    let staged = hermetic(
        std::process::Command::new("git")
            .args(["add", "f.txt"])
            .current_dir(tmp.path()),
    )
    .status()
    .unwrap()
    .success();
    assert!(staged, "failed to stage the test file");

    let output = super::hardened_git(tmp.path())
        .args(["commit", "-m", "msg"])
        .output()
        .await
        .unwrap();

    assert!(
        output.status.success(),
        "commit should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !marker.exists(),
        "hardened_git ran the repository-configured pre-commit hook — the \
         override that is supposed to hold even without the allowlist check \
         did not"
    );
}

/// `commit.gpgsign` is on `ALLOWED_REPO_CONFIG` as an ordinary boolean, but
/// left un-neutralised it would let a repository force every commit through
/// this tool to be signed. `output.status.success()` alone does not prove
/// that: a repository that also configures a *working* `gpg.program` would
/// make a signed commit succeed too, so this plants a fake one that always
/// signs successfully and then inspects the commit object itself for a
/// `gpgsig` header — the only assertion that actually distinguishes "signing
/// was skipped" from "signing was attempted and happened to work".
#[cfg(unix)]
#[tokio::test]
async fn hardened_git_neutralises_forced_commit_signing() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    set_config(tmp.path(), "user.email", "test@example.com");
    set_config(tmp.path(), "user.name", "Test");
    set_config(tmp.path(), "commit.gpgsign", "true");

    // A `gpg.program` that always "signs" successfully, so a regression here
    // fails by finding a signature, not by the commit merely erroring out —
    // the same distinction CodeRabbit's review raised.
    let fake_gpg = tmp.path().join("fake-gpg.sh");
    std::fs::write(
        &fake_gpg,
        "#!/bin/sh\n\
         printf '%s\\n' '[GNUPG:] BEGIN_SIGNING H10' >&2\n\
         cat >/dev/null\n\
         printf -- '-----BEGIN PGP SIGNATURE-----\\n\\nZmFrZQ==\\n-----END PGP SIGNATURE-----\\n'\n\
         printf '%s\\n' '[GNUPG:] SIG_CREATED D 1 10 00 0 0123456789ABCDEF' >&2\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake_gpg, std::fs::Permissions::from_mode(0o755)).unwrap();
    set_config(tmp.path(), "gpg.program", &fake_gpg.to_string_lossy());

    std::fs::write(tmp.path().join("f.txt"), "hi").unwrap();
    let staged = hermetic(
        std::process::Command::new("git")
            .args(["add", "f.txt"])
            .current_dir(tmp.path()),
    )
    .status()
    .unwrap()
    .success();
    assert!(staged, "failed to stage the test file");

    let output = super::hardened_git(tmp.path())
        .args(["commit", "-m", "msg"])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "commit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let commit_object = super::hardened_git(tmp.path())
        .args(["cat-file", "-p", "HEAD"])
        .output()
        .await
        .unwrap();
    assert!(commit_object.status.success());
    let commit_object = String::from_utf8_lossy(&commit_object.stdout);
    assert!(
        !commit_object.lines().any(|l| l.starts_with("gpgsig ")),
        "commit.gpgsign=true must not be honoured, but HEAD carries a \
         signature: {commit_object}"
    );
}

/// The config-inspection step must fail closed: if `git config --list
/// --local` cannot be read, that is not the same as "nothing to distrust",
/// and running the real command anyway would skip the check entirely.
#[cfg(unix)]
#[tokio::test]
async fn unreadable_repo_config_fails_closed_rather_than_running_anyway() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    let config_path = tmp.path().join(".git").join("config");
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o000)).unwrap();

    // Root (and some CI containers run as root) ignores this permission bit
    // entirely, which would make the assertion below meaningless rather than
    // wrong. Detect that up front instead of failing on an unrelated cause.
    let permission_enforced = std::fs::File::open(&config_path).is_err();

    let result = if permission_enforced {
        let tool = test_tool(tmp.path());
        Some(tool.execute(json!({"operation": "status"})).await)
    } else {
        None
    };

    // Restore permissions before the TempDir is dropped, so cleanup doesn't
    // fail on an unreadable file.
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let Some(result) = result else {
        eprintln!(
            "skipping: file permissions are not enforced against this process (running as root?)"
        );
        return;
    };
    let msg = error_text(&result);

    assert!(
        msg.contains("could not inspect its repository config"),
        "an unreadable repo config must refuse, not silently proceed, got: {msg}"
    );
}

#[test]
fn a_subsection_is_elided_so_one_entry_covers_every_remote() {
    assert_eq!(normalise_config_key("remote.origin.url"), "remote.url");
    assert_eq!(normalise_config_key("remote.a.b.c.url"), "remote.url");
    assert_eq!(normalise_config_key("core.fileMode"), "core.filemode");
    assert_eq!(normalise_config_key("core.fsmonitor"), "core.fsmonitor");
    // The subsection itself contains dots; the first and last components
    // remain the reliable ones.
    assert_eq!(
        normalise_config_key("includeIf.gitdir:~/x.y/.path"),
        "includeif.path"
    );
    // A key with no dot at all is returned unchanged rather than panicking.
    assert_eq!(normalise_config_key("bare"), "bare");
}

// ── External diff suppression ─────────────────────────────────────────────

/// An ordinary repository's `diff` must produce its patch.
///
/// That reads like it could not possibly regress, and it did. Suppression of
/// an external diff was attempted with `-c diff.external=` in
/// `NEUTRALISED_CONFIG`, and an empty value does not disable one — git tries
/// to *execute* the empty string, so **every** diff died with
/// `error: cannot run : No such file or directory` /
/// `fatal: external diff died`, on every repository, hostile or not. The
/// hardening removed the operation instead of hardening it. `--no-ext-diff`
/// on the diff command is the real suppression.
///
/// This lives in the lib suite deliberately. The integration test that caught
/// it sits in `raw_coverage_all`, and a change to `git_operations.rs` maps to
/// the `openhuman::tools` libtest filter — which never selects that target. So
/// the lane the change picks did not run the test covering the change, and the
/// breakage sat until an unrelated PR happened to select the other filter.
/// A test here runs whenever this file is touched.
///
/// A repository that *sets* `diff.external` is a separate matter and is
/// already refused before reaching the invocation: the key is on neither
/// allowlist, so `first_disallowed_repo_config_key` rejects the repository
/// outright. `--no-ext-diff` is the second layer, covering the gap between
/// that inspection and the command.
#[tokio::test]
async fn an_ordinary_repository_still_produces_a_diff() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());

    // A committer identity has to be set in the repository itself. `hermetic`
    // closes the global and system config, which is the point of it — so on a
    // CI container with no identity of its own `git commit` fails with
    // "Author identity unknown". Both keys are on `ALLOWED_REPO_CONFIG`, so
    // setting them does not trip the repository-config refusal.
    set_config(tmp.path(), "user.email", "test@example.invalid");
    set_config(tmp.path(), "user.name", "Test");

    let tracked = tmp.path().join("tracked.txt");
    std::fs::write(&tracked, "first\n").unwrap();
    // `hermetic` closes the ambient git config the way the fixtures do —
    // without it a developer's global `commit.gpgsign` or hooks path can fail
    // this commit for reasons unrelated to the test.
    // Slices, not arrays: `["add", "tracked.txt"]` and `["commit", "-m", "one"]`
    // are `[&str; 2]` and `[&str; 3]`, which are different types and cannot
    // share an array literal.
    for args in [&["add", "tracked.txt"][..], &["commit", "-m", "one"][..]] {
        let ok = hermetic(
            std::process::Command::new("git")
                .args(args)
                .current_dir(tmp.path()),
        )
        .status()
        .unwrap()
        .success();
        assert!(ok, "failed to run `git {}` in the test workspace", args[0]);
    }
    std::fs::write(&tracked, "first\nsecond\n").unwrap();

    let tool = test_tool(tmp.path());
    let result = tool
        .execute(json!({"operation": "diff", "files": "tracked.txt"}))
        .await
        .expect("a diff on a plain repository must not error out");

    assert!(!result.is_error, "got: {}", result.output());
    assert!(
        result.output().contains("second"),
        "the added line must appear in the patch: {}",
        result.output()
    );
    assert!(
        !result.output().contains("external diff died"),
        "git must not be handed an empty `diff.external` to execute: {}",
        result.output()
    );
}
