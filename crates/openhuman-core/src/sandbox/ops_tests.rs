use super::*;
use crate::config::{RuntimeConfig, SandboxConfig};
use crate::core::runtime::DispatchCtx;
use crate::sandbox::operator::SANDBOX_OFF_ENV;
use crate::sandbox::spawn_policy;

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
    let policy = unsandboxed_policy(Path::new("/tmp"), Path::new("/tmp/state"));
    assert_eq!(policy.backend, SandboxBackendKind::None);
    let handle = create_sandbox_backend(&DispatchCtx::process_default(), &policy).await;
    assert_eq!(handle.kind, SandboxBackendKind::None);
    assert_eq!(handle.status, SandboxStatus::Ready);
}

/// A policy naming no executor whose state dir is a throwaway directory.
fn unsandboxed_policy(workspace: &Path, state: &Path) -> SandboxPolicy {
    spawn_policy(
        workspace,
        state,
        &SandboxConfig::default(),
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
    execute_in_sandbox(
        &crate::core::runtime::DispatchCtx::host_default(),
        &policy,
        command,
        workspace,
        env.clone(),
        timeout,
    )
    .await
}

// The `/tmp` path and Unix builtins (`false`) are Unix-only, so these
// integration-style tests are gated to Unix. A cross-platform
// `run_on_host_echo_runs_on_every_os` below exercises the same
// code path on Windows CI (#4705) — that is the primary regression
// guard for the `sh` → platform-aware shell fix.
#[cfg(unix)]
#[tokio::test]
async fn run_on_host_echo() {
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
async fn run_on_host_failure() {
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

/// #4705 regression — every OS. `run_on_host` used to
/// `Command::new("sh")`, which fails at `CreateProcessW` on Windows
/// in ~30ms because `sh` is not in PATH. `echo hello` and `exit 1`
/// are shell builtins on both `cmd.exe` and `sh`/`bash`, so this
/// exercises the real code path on Windows CI as well as Unix.
#[tokio::test]
async fn run_on_host_echo_runs_on_every_os() {
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

/// Environment overrides cross the module bus as JSON strings, so a value that
/// is not UTF-8 is refused outright rather than altered on the way.
#[cfg(unix)]
#[tokio::test]
async fn run_on_host_refuses_non_utf8_environment_instead_of_altering_it() {
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
async fn run_on_host_applies_environment_overrides() {
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
// `run_on_host` and the TinyBox native jail adapter both start from the module's safe
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
// ── The operator's switch is not a host decision ────────────────────

/// `[components] sandbox = false` no longer short-circuits the routing: a sandboxed
/// command still reaches the module, which alone decides whether the jail is skipped.
#[tokio::test]
async fn the_sandbox_component_switch_does_not_route_a_sandboxed_command_around_the_module() {
    let mut set = crate::core::runtime::ComponentSet::all();
    set.sandbox = false;
    let ctx = crate::core::runtime::context::CoreContext::for_test_with_components(set);
    let dispatch = DispatchCtx::new(ctx.clone());
    crate::core::runtime::context::CoreContext::scope(ctx, async {
        assert!(
            command_requires_sandbox(&dispatch, Some(SandboxMode::Sandboxed))
                .await
                .unwrap()
        );
    })
    .await;
}

// ── SaaS routing (issue #7328 phase-0 characterization) ──────────────

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

#[cfg(feature = "security-module")]
fn local_policy(workspace: &Path) -> SandboxPolicy {
    SandboxPolicy {
        backend: SandboxBackendKind::Local,
        workspace_root: workspace.to_path_buf(),
        state_dir: workspace.join("state"),
        read_only_mounts: Vec::new(),
        read_write_mounts: Vec::new(),
        allow_network: false,
        env_passthrough: Vec::new(),
        docker_overrides: None,
        mount_workspace: true,
    }
}

/// A sandboxed spawn the module cannot plan is refused with a typed plan error,
/// never run on a host-side decision.
#[cfg(feature = "security-module")]
#[tokio::test]
async fn a_sandboxed_spawn_the_module_cannot_plan_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;
    let error = admit_spawn(
        &config,
        &local_policy(dir.path()),
        "echo hi",
        dir.path(),
        &HashMap::new(),
        false,
    )
    .await
    .expect_err("no module, no plan");
    // A fault is not a deterministic refusal: it must not be mistaken for the floor.
    assert!(
        error.downcast_ref::<IsolationFloor>().is_none(),
        "{error:#}"
    );
}

/// The operation a plan binds must cross the bus as text, so an environment that
/// is not UTF-8 is refused before the module is asked.
#[cfg(all(feature = "security-module", unix))]
#[tokio::test]
async fn a_sandboxed_spawn_with_a_non_text_environment_is_refused_before_planning() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let config = crate::config::Config::default();
    let mut env = HashMap::new();
    env.insert(
        OsString::from("NAME"),
        OsString::from(std::ffi::OsStr::from_bytes(b"\xff")),
    );
    let error = admit_spawn(
        &config,
        &local_policy(dir.path()),
        "echo hi",
        dir.path(),
        &env,
        false,
    )
    .await
    .expect_err("a non-text environment cannot be planned");
    assert!(error.to_string().contains("UTF-8"), "{error:#}");
}
