use super::*;
use crate::config::RuntimeConfig;

#[test]
fn resolve_sandbox_policy_none_mode() {
    let policy = resolve_sandbox_policy(
        SandboxMode::None,
        Path::new("/tmp/action"),
        Path::new("/tmp/state"),
        &RuntimeConfig::default(),
        false,
    );
    assert_eq!(policy.backend, SandboxBackendKind::None);
}

#[test]
fn resolve_sandbox_policy_read_only_mode() {
    let policy = resolve_sandbox_policy(
        SandboxMode::ReadOnly,
        Path::new("/tmp/action"),
        Path::new("/tmp/state"),
        &RuntimeConfig::default(),
        false,
    );
    assert_eq!(policy.backend, SandboxBackendKind::None);
}

#[test]
fn resolve_sandbox_policy_sandboxed_local() {
    let policy = resolve_sandbox_policy(
        SandboxMode::Sandboxed,
        Path::new("/tmp/action"),
        Path::new("/tmp/state"),
        &RuntimeConfig::default(),
        false,
    );
    assert_eq!(policy.backend, SandboxBackendKind::Local);
    assert!(policy.allow_network);
}

#[test]
fn resolve_sandbox_policy_sandboxed_remote_uses_docker() {
    let policy = resolve_sandbox_policy(
        SandboxMode::Sandboxed,
        Path::new("/tmp/action"),
        Path::new("/tmp/state"),
        &RuntimeConfig::default(),
        true,
    );
    assert_eq!(policy.backend, SandboxBackendKind::Docker);
    assert!(!policy.allow_network);
    assert!(policy.docker_overrides.is_some());
}

#[test]
fn resolve_sandbox_policy_docker_runtime_forces_docker() {
    let config = RuntimeConfig {
        kind: "docker".into(),
        ..RuntimeConfig::default()
    };
    let policy = resolve_sandbox_policy(
        SandboxMode::Sandboxed,
        Path::new("/tmp/action"),
        Path::new("/tmp/state"),
        &config,
        false,
    );
    assert_eq!(policy.backend, SandboxBackendKind::Docker);
    assert!(policy.allow_network);
}

#[test]
fn is_elevated_op_known_tools() {
    assert!(is_elevated_op("git_operations"));
    assert!(is_elevated_op("install_tool"));
    assert!(!is_elevated_op("shell"));
    assert!(!is_elevated_op("file_read"));
}

#[test]
fn build_elevated_op_creates_record() {
    let op = build_elevated_op("git_operations", "git push", "VCS requires host access");
    assert_eq!(op.tool_name, "git_operations");
    assert_eq!(op.command, "git push");
    assert!(op.reason.contains("VCS"));
}

#[tokio::test]
async fn create_sandbox_backend_none() {
    let policy = resolve_sandbox_policy(
        SandboxMode::None,
        Path::new("/tmp"),
        Path::new("/tmp/state"),
        &RuntimeConfig::default(),
        false,
    );
    let handle = create_sandbox_backend(&policy).await;
    assert_eq!(handle.kind, SandboxBackendKind::None);
    assert_eq!(handle.status, SandboxStatus::Ready);
}

#[tokio::test]
async fn create_sandbox_backend_local() {
    let policy = resolve_sandbox_policy(
        SandboxMode::Sandboxed,
        Path::new("/tmp"),
        Path::new("/tmp/state"),
        &RuntimeConfig::default(),
        false,
    );
    let handle = create_sandbox_backend(&policy).await;
    assert_eq!(handle.kind, SandboxBackendKind::Local);

    // Assert the BACKEND -> STATUS pairing, not a fixed value. This test
    // previously asserted `Ready` unconditionally, which is precisely the
    // defect being fixed: on a host with no OS jail, `pick_backend` falls back
    // to `NoopBackend` and the handle claimed the sandbox was ready while
    // commands ran unconfined. Which branch runs here depends on the CI host,
    // so pin the relationship instead of the outcome.
    let backend_id = handle
        .backend_id
        .as_deref()
        .expect("the local backend must name itself so a caller can tell which jail is in force");
    if backend_id == cwd_jail::NOOP_BACKEND_NAME {
        assert_eq!(
            handle.status,
            SandboxStatus::Inactive,
            "the noop passthrough enforces nothing, so it must not report `Ready`"
        );
    } else {
        assert_eq!(
            handle.status,
            SandboxStatus::Ready,
            "a real OS jail ({backend_id}) is in force, so `Ready` is honest"
        );
    }
}

// The `/tmp` path and Unix builtins (`false`) are Unix-only, so these
// integration-style tests are gated to Unix. A cross-platform
// `execute_unsandboxed_echo_runs_on_every_os` below exercises the same
// code path on Windows CI (#4705) — that is the primary regression
// guard for the `sh` → platform-aware shell fix.
#[cfg(unix)]
#[tokio::test]
async fn execute_unsandboxed_echo() {
    let result = execute_unsandboxed(
        "echo hello",
        Path::new("/tmp"),
        &HashMap::new(),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("hello"));
    assert!(!result.timed_out);
}

#[cfg(unix)]
#[tokio::test]
async fn execute_unsandboxed_failure() {
    let result = execute_unsandboxed(
        "false",
        Path::new("/tmp"),
        &HashMap::new(),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_ne!(result.exit_code, 0);
}

/// #4705 regression — every OS. `execute_unsandboxed` used to
/// `Command::new("sh")`, which fails at `CreateProcessW` on Windows
/// in ~30ms because `sh` is not in PATH. `echo hello` and `exit 1`
/// are shell builtins on both `cmd.exe` and `sh`/`bash`, so this
/// exercises the real code path on Windows CI as well as Unix.
#[tokio::test]
async fn execute_unsandboxed_echo_runs_on_every_os() {
    let tempdir = tempfile::tempdir().unwrap();
    let result = execute_unsandboxed(
        "echo hello",
        tempdir.path(),
        &HashMap::new(),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(result.stdout.contains("hello"));
    assert!(!result.timed_out);

    let failing = execute_unsandboxed(
        "exit 1",
        tempdir.path(),
        &HashMap::new(),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_ne!(failing.exit_code, 0);
}

#[cfg(unix)]
#[tokio::test]
async fn execute_in_sandbox_none_backend() {
    let policy = resolve_sandbox_policy(
        SandboxMode::None,
        Path::new("/tmp"),
        Path::new("/tmp/state"),
        &RuntimeConfig::default(),
        false,
    );
    let result = execute_in_sandbox(
        &policy,
        "echo sandbox-test",
        Path::new("/tmp"),
        HashMap::new(),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert!(result.success());
    assert!(result.stdout.contains("sandbox-test"));
}

#[cfg(unix)]
#[tokio::test]
async fn execute_in_sandbox_preserves_non_utf8_environment_bytes() {
    use std::os::unix::ffi::OsStringExt;

    let tempdir = tempfile::tempdir().unwrap();
    let policy = resolve_sandbox_policy(
        SandboxMode::None,
        tempdir.path(),
        Path::new("/tmp/state"),
        &RuntimeConfig::default(),
        false,
    );
    let env = HashMap::from([(
        OsString::from("OPENHUMAN_RAW_ENV_TEST"),
        OsString::from_vec(b"before-\xff-after".to_vec()),
    )]);
    let result = execute_in_sandbox(
        &policy,
        r#"[ "$OPENHUMAN_RAW_ENV_TEST" = "$(printf 'before-\377-after')" ]"#,
        tempdir.path(),
        env,
        Duration::from_secs(10),
    )
    .await
    .unwrap();

    assert!(result.success(), "stderr: {}", result.stderr);
}

/// #4705 regression — `execute_in_sandbox` with the `None` backend
/// now delegates to `execute_unsandboxed`, which used to fail on
/// Windows with a ~30ms `sh`-not-found spawn error. Cross-platform
/// so both Unix and Windows CI catch a shell-selection regression.
#[tokio::test]
async fn execute_in_sandbox_none_backend_runs_on_every_os() {
    let tempdir = tempfile::tempdir().unwrap();
    let policy = resolve_sandbox_policy(
        SandboxMode::None,
        tempdir.path(),
        Path::new("/tmp/state"),
        &RuntimeConfig::default(),
        false,
    );
    let result = execute_in_sandbox(
        &policy,
        "echo sandbox-test",
        tempdir.path(),
        HashMap::new(),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert!(result.success(), "stderr: {}", result.stderr);
    assert!(result.stdout.contains("sandbox-test"));
}

#[test]
fn env_passthrough_includes_safe_vars() {
    assert!(SANDBOX_ENV_PASSTHROUGH.contains(&"PATH"));
    assert!(SANDBOX_ENV_PASSTHROUGH.contains(&"HOME"));
    assert!(!SANDBOX_ENV_PASSTHROUGH
        .iter()
        .any(|v| v.contains("KEY") || v.contains("SECRET")));
}

// ── the security-relevant decision, tested independently of the host ─────────
//
// `create_sandbox_backend_local` above can only exercise whichever branch this
// machine happens to take. On a host with Seatbelt or Landlock it never reaches
// the noop path, so a regression to "always Ready" would pass there unnoticed —
// which is how the original defect survived. These pin the decision directly.

#[test]
fn local_status_is_inactive_when_no_os_jail_is_in_force() {
    assert_eq!(
        local_status_for_backend(cwd_jail::NOOP_BACKEND_NAME),
        SandboxStatus::Inactive,
        "the noop backend enforces nothing; reporting `Ready` tells a caller its \
         commands are jailed when they run unconfined"
    );
}

#[test]
fn local_status_is_ready_for_a_real_jail() {
    for backend in ["seatbelt", "landlock", "appcontainer"] {
        assert_eq!(
            local_status_for_backend(backend),
            SandboxStatus::Ready,
            "a real OS jail ({backend}) is in force, so `Ready` is honest"
        );
    }
}

// ── #6961: local-jail output capture stays out of the user's project ─────────

#[cfg(unix)]
fn local_policy(action_dir: &Path, state_dir: &Path) -> SandboxPolicy {
    let policy = resolve_sandbox_policy(
        SandboxMode::Sandboxed,
        action_dir,
        state_dir,
        &RuntimeConfig::default(),
        false,
    );
    assert_eq!(policy.backend, SandboxBackendKind::Local);
    policy
}

#[cfg(unix)]
async fn run_local(policy: &SandboxPolicy, command: &str) -> SandboxExecResult {
    execute_in_sandbox(
        policy,
        command,
        &policy.workspace_root,
        HashMap::new(),
        Duration::from_secs(20),
    )
    .await
    .unwrap()
}

#[cfg(unix)]
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[cfg(unix)]
#[tokio::test]
async fn local_jail_writes_no_capture_files_into_the_workspace_root() {
    let action = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let policy = local_policy(action.path(), state.path());

    // `ls -A` runs inside the root while the capture is live, so it sees
    // anything the capture put there (this is what `git status` saw).
    let during = run_local(&policy, "ls -A; echo to-stderr >&2").await;

    assert!(during.success(), "stderr: {}", during.stderr);
    assert_eq!(during.stdout, "", "root was not empty while running");
    assert_eq!(during.stderr, "to-stderr\n");
    assert!(
        entries(action.path()).is_empty(),
        "root was not empty after"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn concurrent_local_jail_runs_keep_their_outputs_separate() {
    let action = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let policy = local_policy(action.path(), state.path());

    let (a, b) = tokio::join!(
        run_local(&policy, "echo a1; echo a-err >&2; sleep 0.4; echo a2"),
        run_local(&policy, "echo b1; echo b-err >&2; sleep 0.4; echo b2"),
    );

    assert_eq!(
        (a.stdout.as_str(), a.stderr.as_str()),
        ("a1\na2\n", "a-err\n")
    );
    assert_eq!(
        (b.stdout.as_str(), b.stderr.as_str()),
        ("b1\nb2\n", "b-err\n")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn local_jail_captures_under_the_state_dir_and_removes_the_call_dir() {
    let action = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut policy = local_policy(action.path(), state.path());
    let capture_root = sandbox_capture_root(state.path());
    assert_eq!(
        capture_root,
        state.path().join("artifacts").join("sandbox-capture")
    );
    // A real jail grants only the per-call dir, so the command cannot list the
    // capture root on its own; grant it read-only so this test can look.
    std::fs::create_dir_all(&capture_root).unwrap();
    policy.read_only_mounts.push(capture_root.clone());

    // While running, exactly one per-call dir holding both streams exists.
    let during = run_local(&policy, &format!("ls '{}'/*", capture_root.display())).await;
    assert!(during.success(), "stderr: {}", during.stderr);
    assert_eq!(during.stdout, "stderr\nstdout\n");

    assert!(
        entries(&capture_root).is_empty(),
        "per-call capture dir left behind: {:?}",
        entries(&capture_root)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn local_jail_removes_the_call_dir_when_the_spawn_fails() {
    let cwd = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    // A jail root that does not exist makes the jail refuse to spawn.
    let policy = local_policy(&cwd.path().join("missing-root"), state.path());

    let err = execute_in_sandbox(
        &policy,
        "true",
        cwd.path(),
        HashMap::new(),
        Duration::from_secs(5),
    )
    .await
    .unwrap_err();

    assert!(
        err.to_string().contains("Failed to spawn jailed process"),
        "{err}"
    );
    assert!(entries(&sandbox_capture_root(state.path())).is_empty());
}

// ── tinybox `unsupported` backend is not a jail ──────────────────────────────

#[test]
fn local_status_is_inactive_for_the_unsupported_backend() {
    assert_eq!(
        local_status_for_backend(cwd_jail::detect::UNSUPPORTED_BACKEND_NAME),
        SandboxStatus::Inactive,
        "`pick_backend` answers `unsupported` when no OS jail exists and the host then runs \
         commands through the no-op fallback; `Ready` would claim confinement that is absent"
    );
}

// ── tinybox#23: with a real jail, everyday commands still work ───────────────
//
// These run a real Landlock jail and skip (loudly) where the kernel has none,
// so they never fail a host that cannot confine.

#[cfg(target_os = "linux")]
fn landlock_in_force() -> bool {
    let backend = cwd_jail::default_backend();
    let ok = backend.name() == "landlock" && backend.is_available();
    if !ok {
        eprintln!(
            "SKIP: no Landlock on this host (backend = {})",
            backend.name()
        );
    }
    ok
}

#[cfg(target_os = "linux")]
fn host_has(program: &str) -> bool {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {program}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn landlock_jail_runs_cargo_and_mktemp_but_blocks_writes_outside() {
    if !landlock_in_force() {
        return;
    }
    let action = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let policy = local_policy(action.path(), state.path());

    if host_has("cargo") {
        let r = run_local(&policy, "cargo --version").await;
        assert!(r.success(), "cargo failed under the jail: {}", r.stderr);
        assert!(r.stdout.starts_with("cargo "), "stdout: {}", r.stdout);
    } else {
        eprintln!("SKIP cargo: not installed on this host");
    }

    // `/tmp` is not granted; `mktemp` lands in the per-call TMPDIR scratch dir.
    let r = run_local(&policy, "mktemp").await;
    assert!(r.success(), "mktemp failed under the jail: {}", r.stderr);
    assert!(
        r.stdout
            .trim()
            .starts_with(sandbox_scratch_root(state.path()).to_str().unwrap()),
        "mktemp should land in the scratch dir, got {:?}",
        r.stdout
    );
    assert!(
        entries(&sandbox_scratch_root(state.path())).is_empty(),
        "per-call scratch dir left behind"
    );

    // The jail still confines: writing outside the root fails.
    let target = outside.path().join("pwned");
    let r = run_local(&policy, &format!("echo x > '{}'", target.display())).await;
    assert!(!r.success(), "write outside the root must fail");
    assert!(!target.exists(), "the jail let a write escape");

    // ...while the workspace root itself stays writable.
    let r = run_local(&policy, "echo ok > inside.txt").await;
    assert!(r.success(), "stderr: {}", r.stderr);
    assert!(action.path().join("inside.txt").exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn landlock_jail_denies_proc_unless_the_toggle_is_on() {
    if !landlock_in_force() {
        return;
    }
    let action = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let policy = local_policy(action.path(), state.path());
    let denied = run_local(&policy, "cat /proc/self/environ > /dev/null").await;
    assert!(!denied.success(), "/proc must be off by default");

    let config = RuntimeConfig {
        local_jail: crate::config::LocalJailConfig {
            allow_proc: true,
            ..Default::default()
        },
        ..RuntimeConfig::default()
    };
    let policy = resolve_sandbox_policy(
        SandboxMode::Sandboxed,
        action.path(),
        state.path(),
        &config,
        false,
    );
    let allowed = run_local(&policy, "cat /proc/self/environ > /dev/null").await;
    assert!(allowed.success(), "stderr: {}", allowed.stderr);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn landlock_jail_cannot_read_the_users_ssh_directory() {
    if !landlock_in_force() {
        return;
    }
    let Some(ssh) = dirs::home_dir()
        .map(|h| h.join(".ssh"))
        .filter(|p| p.is_dir())
    else {
        eprintln!("SKIP: no ~/.ssh on this host");
        return;
    };
    let action = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let policy = local_policy(action.path(), state.path());
    let r = run_local(&policy, &format!("ls '{}'", ssh.display())).await;
    assert!(!r.success(), "~/.ssh must stay unreachable from the jail");
}

#[cfg(unix)]
#[tokio::test]
async fn local_handle_status_matches_the_backend_actually_in_force() {
    let action = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let handle = create_sandbox_backend(&local_policy(action.path(), state.path())).await;
    let name = handle.backend_id.clone().unwrap();
    let unconfined =
        name == cwd_jail::NOOP_BACKEND_NAME || name == cwd_jail::detect::UNSUPPORTED_BACKEND_NAME;
    assert_eq!(
        handle.status,
        if unconfined {
            SandboxStatus::Inactive
        } else {
            SandboxStatus::Ready
        },
        "backend = {name}"
    );
}
