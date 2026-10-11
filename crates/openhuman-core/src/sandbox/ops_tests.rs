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

// ── Windows child environment ────────────────────────────────────────────────
//
// `execute_unsandboxed` and the TinyBox native jail adapter both call `env_clear()` and
// re-forward only `SANDBOX_ENV_PASSTHROUGH`. On Windows that list has to carry
// the process-bootstrap variables or the child cannot initialise the OS crypto
// provider — `node` dies with `Assertion failed: ncrypto::CSPRNG(nullptr, 0)`
// (exit 134) and `powershell` with `8009001d`. Both read as opaque failures to
// the agent, which is how the original defect shipped: the four tool launchers'
// allow-lists were fixed, the sandbox allow-list — the one the `sandboxed`
// orchestrator actually routes through — was not.

// What a real child actually receives is covered end-to-end in
// `tests/windows_sandbox_env_e2e.rs`, which spawns through `execute_in_sandbox`
// and asserts on the child's own environment. It lives there because the probe
// has to be a Rust `CreateProcess` spawn: `node`'s own `child_process.spawn`
// silently injects `SystemRoot`, so a JavaScript probe reports a stripped
// environment as healthy.
#[test]
fn sandbox_off_value_accepts_off_spellings() {
    for v in ["off", "OFF", " none ", "0", "false", "Disabled"] {
        assert!(
            sandbox_off_value(Some(v)),
            "{v:?} should disable the sandbox"
        );
    }
}

#[test]
fn sandbox_off_value_keeps_sandbox_on_otherwise() {
    for v in [
        None,
        Some(""),
        Some("on"),
        Some("1"),
        Some("true"),
        Some("local"),
    ] {
        assert!(!sandbox_off_value(v), "{v:?} must leave the sandbox on");
    }
}
