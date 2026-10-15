use super::*;
use crate::config::RuntimeConfig;
use crate::core::runtime::DispatchCtx;

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
    let handle = create_sandbox_backend(&DispatchCtx::process_default(), &policy).await;
    assert_eq!(handle.kind, SandboxBackendKind::None);
    assert_eq!(handle.status, SandboxStatus::Ready);
}

/// A `None`-backend policy whose state dir is a throwaway directory.
fn unsandboxed_policy(workspace: &Path, state: &Path) -> SandboxPolicy {
    resolve_sandbox_policy(
        SandboxMode::None,
        workspace,
        state,
        &RuntimeConfig::default(),
        false,
    )
}

async fn run_unsandboxed(
    command: &str,
    workspace: &Path,
    env: &HashMap<OsString, OsString>,
    timeout: Duration,
) -> anyhow::Result<SandboxExecResult> {
    let state = tempfile::tempdir()?;
    let policy = unsandboxed_policy(workspace, state.path());
    execute_unsandboxed(
        &crate::config::Config::default(),
        &policy,
        command,
        workspace,
        env,
        timeout,
        None,
    )
    .await
}

// The `/tmp` path and Unix builtins (`false`) are Unix-only, so these
// integration-style tests are gated to Unix. A cross-platform
// `execute_unsandboxed_echo_runs_on_every_os` below exercises the same
// code path on Windows CI (#4705) — that is the primary regression
// guard for the `sh` → platform-aware shell fix.
#[cfg(unix)]
#[tokio::test]
async fn execute_unsandboxed_echo() {
    let result = run_unsandboxed(
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
    let result = run_unsandboxed(
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
    let result = run_unsandboxed(
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

    let failing = run_unsandboxed(
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
        &DispatchCtx::process_default(),
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

/// Environment overrides cross the module bus as JSON strings, so a value that
/// is not UTF-8 is refused outright rather than altered on the way.
#[cfg(unix)]
#[tokio::test]
async fn execute_unsandboxed_refuses_non_utf8_environment_instead_of_altering_it() {
    use std::os::unix::ffi::OsStringExt;

    let tempdir = tempfile::tempdir().unwrap();
    let env = HashMap::from([(
        OsString::from("OPENHUMAN_RAW_ENV_TEST"),
        OsString::from_vec(b"before-\xff-after".to_vec()),
    )]);
    let marker = tempdir.path().join("must-not-run");
    let result = run_unsandboxed(
        &format!("touch '{}'", marker.display()),
        tempdir.path(),
        &env,
        Duration::from_secs(10),
    )
    .await;

    assert!(result.is_err());
    assert!(
        !marker.exists(),
        "a refused environment must not run the command"
    );
}

/// Caller-approved overrides reach the command on top of the safe snapshot.
#[cfg(unix)]
#[tokio::test]
async fn execute_unsandboxed_applies_environment_overrides() {
    let tempdir = tempfile::tempdir().unwrap();
    let env = HashMap::from([(
        OsString::from("OPENHUMAN_OVERRIDE_TEST"),
        OsString::from("applied"),
    )]);
    let result = run_unsandboxed(
        "printf %s \"$OPENHUMAN_OVERRIDE_TEST\"",
        tempdir.path(),
        &env,
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(result.stdout, "applied");
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
        &DispatchCtx::process_default(),
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
// `execute_unsandboxed` and the TinyBox native jail adapter both start from the module's safe
// environment snapshot (`SANDBOX_ENV_PASSTHROUGH` plus the Windows bootstrap names). On Windows that list has to carry
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

// ── [components] sandbox = false ──────────────────────────────────────

#[tokio::test]
async fn sandbox_off_component_forces_the_unsandboxed_fallback_and_no_module_load() {
    let mut set = crate::core::runtime::ComponentSet::all();
    set.sandbox = false;
    let ctx = crate::core::runtime::context::CoreContext::for_test_with_components(set);
    let dispatch = DispatchCtx::new(ctx.clone());
    crate::core::runtime::context::CoreContext::scope(ctx, async {
        assert!(sandbox_component_off());
        assert!(sandbox_disabled_by_host());
        assert!(
            !command_requires_sandbox(&dispatch, Some(SandboxMode::Sandboxed))
                .await
                .unwrap()
        );
    })
    .await;
}

#[tokio::test]
async fn sandbox_component_is_on_without_a_context_or_when_enabled() {
    assert!(!sandbox_component_off());
    let ctx = crate::core::runtime::context::CoreContext::for_test_with_components(
        crate::core::runtime::ComponentSet::all(),
    );
    crate::core::runtime::context::CoreContext::scope(ctx, async {
        assert!(!sandbox_component_off());
    })
    .await;
}

#[test]
fn config_disables_sandbox_for_legacy_flag_or_component() {
    let mut config = crate::config::Config::default();
    assert!(!config_disables_sandbox(&config));
    config.sandbox.enabled = Some(false);
    assert!(config_disables_sandbox(&config));
    config.sandbox.enabled = None;
    config.components.sandbox = false;
    assert!(config_disables_sandbox(&config));
}

// ── SaaS precedence (issue #7328 phase-0 characterization) ────────────

const SAAS_CHILD: &str = "OPENHUMAN_SANDBOX_SAAS_TEST_CHILD";

/// Mode is process-owned and irreversible, so the SaaS assertion runs in a
/// fresh libtest process instead of locking the whole suite into SaaS.
#[tokio::test]
async fn saas_requires_the_sandbox_even_when_the_host_turns_it_off() {
    let module = module_path!().split_once("::").expect("crate module").1;
    let test = format!("{module}::saas_requires_the_sandbox_even_when_the_host_turns_it_off");
    if std::env::var(SAAS_CHILD).as_deref() == Ok(test.as_str()) {
        crate::core::runtime::mode::lock_mode(crate::core::runtime::Mode::Saas).unwrap();
        assert!(crate::core::runtime::is_saas());
        // `OPENHUMAN_SANDBOX=off` is set by the parent. The SaaS rule is checked
        // before the host switch and before any config is read, for every agent mode.
        for mode in [
            None,
            Some(SandboxMode::None),
            Some(SandboxMode::ReadOnly),
            Some(SandboxMode::Sandboxed),
        ] {
            assert!(
                command_requires_sandbox(&DispatchCtx::process_default(), mode)
                    .await
                    .unwrap(),
                "SaaS must sandbox every command, mode {mode:?}"
            );
        }
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", &test, "--nocapture", "--test-threads=1"])
        .env(SAAS_CHILD, &test)
        .env(SANDBOX_OFF_ENV, "off")
        .output()
        .expect("run isolated SaaS test");
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "isolated test {test} failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // The parent process stays single-user: the off switch still wins here.
    assert!(!crate::core::runtime::is_saas());
}

#[test]
fn untrusted_isolation_denies_noop_and_unsuitable_jails() {
    assert!(require_untrusted_isolation(true, true).is_ok());
    assert!(require_untrusted_isolation(true, false).is_err());
    assert!(require_untrusted_isolation(false, true).is_err());
    assert!(require_untrusted_isolation(false, false).is_err());
}
