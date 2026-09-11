use super::*;
use crate::openhuman::config::RuntimeConfig;

#[test]
fn resolve_sandbox_policy_none_mode() {
    let policy = resolve_sandbox_policy(
        SandboxMode::None,
        Path::new("/tmp/action"),
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
