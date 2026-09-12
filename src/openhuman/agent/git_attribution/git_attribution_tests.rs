#[cfg(unix)]
#[test]
fn hook_adds_openhuman_trailer_without_disabling_repository_hook() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "user.email", "test@example.com"]);
    let own_hook = repo.join(".git/hooks/prepare-commit-msg");
    std::fs::write(
        &own_hook,
        "#!/bin/sh\nprintf 'repo-hook-ran\\n' >> \"$1\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&own_hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(repo.join("a"), "a").unwrap();
    git(&["add", "a"]);

    let hook_env = super::hook::test_hook_env(Some(std::ffi::OsStr::new(
        "'test.openhuman-inherited'='kept' 'core.hooksPath'='/definitely-not-the-openhuman-hook'",
    )));
    let output = Command::new("git")
        .args(["commit", "-q", "-m", "subject"])
        .current_dir(&repo)
        .envs(&hook_env)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new("git")
        .args(["log", "-1", "--format=%B"])
        .current_dir(&repo)
        .output()
        .unwrap();
    let message = String::from_utf8(output.stdout).unwrap();
    assert!(message.contains("repo-hook-ran"), "{message:?}");
    assert!(message.contains(super::hook::TRAILER), "{message:?}");

    let inherited = Command::new("git")
        .args(["config", "--get", "test.openhuman-inherited"])
        .current_dir(&repo)
        .envs(&hook_env)
        .output()
        .unwrap();
    assert!(inherited.status.success());
    assert_eq!(String::from_utf8(inherited.stdout).unwrap().trim(), "kept");
}

#[cfg(unix)]
#[test]
fn hook_env_does_not_drop_inherited_parameters_containing_non_utf8() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let inherited_bytes = b"'test.openhuman-inherited'='before-\xff-after' 'test.second'='kept'";
    let inherited = std::ffi::OsStr::from_bytes(inherited_bytes);
    let hook_env = super::hook::test_hook_env(Some(inherited));
    let parameters = hook_env
        .get(std::ffi::OsStr::new("GIT_CONFIG_PARAMETERS"))
        .unwrap()
        .clone()
        .into_vec();

    assert!(parameters.starts_with(inherited_bytes));
    assert_eq!(parameters[inherited_bytes.len()], b' ');
    assert!(parameters[inherited_bytes.len() + 1..].starts_with(b"'core.hooksPath'='"));
    assert!(parameters.contains(&0xff));

    let output = std::process::Command::new("git")
        .args(["config", "--get", "test.openhuman-inherited"])
        .envs(&hook_env)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"before-\xff-after\n");
}

/// A repository-set command-valued key must not reach a shell-spawned `git`.
///
/// `core.fsmonitor` is the probe because it is the one entry of the family
/// that git actually executes with stdout piped: `core.pager` and
/// `core.editor` are skipped without a terminal, so a test built on either
/// would pass whether or not the hardening existed. `git status` runs the
/// fsmonitor program, the repository config is agent-writable, and the
/// program is arbitrary — which is the whole shape of the hole.
///
/// The assertion is on behaviour, not on the env var: a test that only checked
/// `GIT_CONFIG_PARAMETERS` contained the right string would pass just as
/// happily if git ignored it.
#[cfg(unix)]
#[test]
fn a_repository_fsmonitor_does_not_run_under_the_shell_git_environment() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let marker = temp.path().join("fsmonitor-ran");

    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("a"), "a").unwrap();

    // An "fsmonitor" that records that it was executed at all. Exiting
    // non-zero makes git fall back to a normal scan, so `git status` still
    // succeeds either way and the marker is the only signal.
    let hook = temp.path().join("fsmonitor.sh");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    git(&["config", "core.fsmonitor", hook.to_str().unwrap()]);

    // Baseline: without the hardening the repository's program really does
    // run, so the assertion below is discriminating rather than vacuous.
    let bare = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(bare.status.success());
    assert!(
        marker.exists(),
        "the fixture fsmonitor never ran, so this test proves nothing"
    );
    std::fs::remove_file(&marker).unwrap();

    // Now with the environment the shell tool hands every command.
    let hook_env = super::hook::test_hook_env(None);
    let mut cmd = Command::new("git");
    cmd.args(["status", "--porcelain"]).current_dir(&repo);
    for (key, value) in &hook_env {
        cmd.env(key, value);
    }
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "git status failed under the shell git environment: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !marker.exists(),
        "the repository's core.fsmonitor ran under the shell git environment"
    );
}

/// The shell environment must not silently sign commits with the host's key.
#[cfg(unix)]
#[test]
fn the_shell_git_environment_forces_commit_signing_off() {
    let env = super::hook::test_hook_env(None);
    let parameters = env
        .get(std::ffi::OsStr::new("GIT_CONFIG_PARAMETERS"))
        .expect("the shell git environment sets GIT_CONFIG_PARAMETERS")
        .to_string_lossy()
        .into_owned();
    for expected in crate::openhuman::tools::implementations::filesystem::SHELL_NEUTRALISED_CONFIG {
        let (key, value) = expected.split_once('=').unwrap();
        assert!(
            parameters.contains(&format!("'{key}'='{value}'")),
            "`{expected}` is missing from GIT_CONFIG_PARAMETERS: {parameters}"
        );
    }
}
