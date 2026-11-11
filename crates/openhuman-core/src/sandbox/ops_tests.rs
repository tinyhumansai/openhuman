use super::*;
use crate::config::{RuntimeConfig, SandboxConfig};
use crate::core::runtime::DispatchCtx;
use crate::sandbox::{
    shell_exec::ShellRunOutcome,
    types::{SandboxBackendKind, SandboxPolicy, SandboxStatus},
    SandboxExecResult,
};
use std::path::Path;
use std::time::Duration;
use std::{collections::HashMap, ffi::OsString};
use tinyagents_definition::SandboxMode;

/// The module host serves a TinyBox resource's records from the process
/// storage slot. A unit test does not boot the core, so it installs one: with
/// no backend the module answers `the host did not provide storage` and a
/// planned command never starts.
fn native_shell_storage() -> crate::storage::test_support::InstalledBackendGuard {
    crate::storage::test_support::install_backend_for_test(
        crate::storage::test_support::memory_backend(),
    )
}

#[test]
fn resolve_sandbox_policy_none_mode() {
    let policy = crate::sandbox::policy::route_for_mode(
        SandboxMode::None,
        Path::new("/tmp/action"),
        Path::new("/tmp/state"),
        &SandboxConfig::default(),
        &RuntimeConfig::default(),
        false,
    );
    assert_eq!(policy.backend, SandboxBackendKind::None);
}

#[test]
fn resolve_sandbox_policy_read_only_mode() {
    let policy = crate::sandbox::policy::route_for_mode(
        SandboxMode::ReadOnly,
        Path::new("/tmp/action"),
        Path::new("/tmp/state"),
        &SandboxConfig::default(),
        &RuntimeConfig::default(),
        false,
    );
    assert_eq!(policy.backend, SandboxBackendKind::None);
}

#[test]
fn resolve_sandbox_policy_sandboxed_local() {
    let policy = crate::sandbox::policy::route_for_mode(
        SandboxMode::Sandboxed,
        Path::new("/tmp/action"),
        Path::new("/tmp/state"),
        &SandboxConfig::default(),
        &RuntimeConfig::default(),
        false,
    );
    assert_eq!(policy.backend, SandboxBackendKind::Local);
    assert!(policy.allow_network);
}

#[test]
fn resolve_sandbox_policy_sandboxed_remote_uses_docker() {
    let policy = crate::sandbox::policy::route_for_mode(
        SandboxMode::Sandboxed,
        Path::new("/tmp/action"),
        Path::new("/tmp/state"),
        &SandboxConfig::default(),
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
    let policy = crate::sandbox::policy::route_for_mode(
        SandboxMode::Sandboxed,
        Path::new("/tmp/action"),
        Path::new("/tmp/state"),
        &SandboxConfig::default(),
        &config,
        false,
    );
    assert_eq!(policy.backend, SandboxBackendKind::Docker);
    assert!(policy.allow_network);
}

#[tokio::test]
async fn create_sandbox_backend_none() {
    let policy = crate::sandbox::policy::route_for_mode(
        SandboxMode::None,
        Path::new("/tmp"),
        Path::new("/tmp/state"),
        &SandboxConfig::default(),
        &RuntimeConfig::default(),
        false,
    );
    let handle = create_sandbox_backend(&DispatchCtx::process_default(), &policy).await;
    assert_eq!(handle.kind, SandboxBackendKind::None);
    assert_eq!(handle.status, SandboxStatus::Ready);
}

/// A `None`-backend policy whose state dir is a throwaway directory.
fn unsandboxed_policy(workspace: &Path, state: &Path) -> SandboxPolicy {
    crate::sandbox::policy::route_for_mode(
        SandboxMode::None,
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
    let outcome = execute_unsandboxed(
        &crate::core::runtime::DispatchCtx::host_default(),
        &crate::config::Config::default(),
        &policy,
        command,
        workspace,
        env,
        timeout,
        None,
        None,
    )
    .await?;
    match outcome {
        ShellRunOutcome::Finished(result) => Ok(result),
        ShellRunOutcome::Running(_) => anyhow::bail!("a run without a yield point kept running"),
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
    let _storage = native_shell_storage();
    // A private folder: the jail's bwrap backend owns `/tmp` as a private mount
    // and refuses a grant on `/tmp` itself.
    let workdir = tempfile::tempdir().unwrap();
    // The module canonicalizes the state folder before it runs a command, so it
    // must exist, as the core's `workspace_dir` does in a booted host.
    let state = tempfile::tempdir().unwrap();
    let policy = crate::sandbox::policy::route_for_mode(
        SandboxMode::None,
        workdir.path(),
        state.path(),
        &SandboxConfig::default(),
        &RuntimeConfig::default(),
        false,
    );
    let result = execute_in_sandbox(
        &DispatchCtx::process_default(),
        &policy,
        "echo sandbox-test",
        workdir.path(),
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
    let _storage = native_shell_storage();
    let tempdir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let policy = crate::sandbox::policy::route_for_mode(
        SandboxMode::None,
        tempdir.path(),
        state.path(),
        &SandboxConfig::default(),
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
        assert!(sandbox_component_off(&dispatch));
        assert!(sandbox_disabled_by_host(&dispatch));
        assert!(crate::sandbox::policy::should_plan_command(
            &dispatch,
            Some(SandboxMode::Sandboxed)
        )
        .await
        .unwrap());
    })
    .await;
}

#[tokio::test]
async fn sandbox_component_is_on_without_a_context_or_when_enabled() {
    assert!(!sandbox_component_off(&DispatchCtx::ambient()));
    let ctx = crate::core::runtime::context::CoreContext::for_test_with_components(
        crate::core::runtime::ComponentSet::all(),
    );
    {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(ctx);

        assert!(!sandbox_component_off(&scope_ctx));
    };
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

#[tokio::test]
async fn the_openshell_handle_is_ready_without_pretending_to_be_a_container() {
    let policy = SandboxPolicy {
        backend: SandboxBackendKind::OpenShell,
        ..unsandboxed_policy(Path::new("/tmp/action"), Path::new("/tmp/state"))
    };

    let handle = create_sandbox_backend_with_config(&policy, &crate::config::Config::default())
        .await
        .unwrap();

    assert_eq!(handle.kind, SandboxBackendKind::OpenShell);
    assert_eq!(handle.status, SandboxStatus::Ready);
    assert_eq!(handle.backend_id, None);
    assert_eq!(handle.isolation, None);
}

#[test]
fn security_disabled_keeps_the_configured_backend_unless_the_operator_turns_it_off() {
    use super::exec::unchecked_backend;
    for backend in [
        SandboxBackendKind::Local,
        SandboxBackendKind::None,
        SandboxBackendKind::Docker,
        SandboxBackendKind::OpenShell,
    ] {
        assert_eq!(unchecked_backend(backend, false), backend);
        assert_eq!(
            unchecked_backend(backend, true),
            SandboxBackendKind::None,
            "the operator's sandbox-off switch still runs on the host"
        );
    }
}

#[cfg(feature = "modules")]
#[test]
fn security_disabled_jails_with_the_policys_own_grants() {
    let mut policy = crate::sandbox::policy::route_for_mode(
        SandboxMode::Sandboxed,
        Path::new("/tmp/action"),
        Path::new("/tmp/state"),
        &SandboxConfig::default(),
        &RuntimeConfig::default(),
        false,
    );
    policy.read_only_mounts = vec!["/opt/tools".into()];
    policy.read_write_mounts = vec!["/srv/out".into()];
    policy.allow_network = false;
    let jail = crate::sandbox::native_jail::host_jail(&policy);
    assert_eq!(jail.read_only, vec!["/opt/tools".to_string()]);
    assert_eq!(jail.read_write, vec!["/srv/out".to_string()]);
    assert!(!jail.allow_net);
    assert!(jail.deny_read.is_empty());
    assert!(!jail.require_process_isolation);
}

fn config_with_backend(backend: crate::config::SandboxBackend) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.sandbox.backend = backend;
    config
}

#[test]
fn a_config_naming_a_box_keeping_backend_keeps_boxes() {
    use crate::config::SandboxBackend;
    for backend in [SandboxBackend::OpenShell, SandboxBackend::Agent37] {
        assert!(
            config_keeps_boxes(&config_with_backend(backend.clone())),
            "{backend:?}"
        );
    }
    for backend in [
        SandboxBackend::Auto,
        SandboxBackend::None,
        SandboxBackend::Docker,
        SandboxBackend::Landlock,
    ] {
        assert!(!config_keeps_boxes(&config_with_backend(backend)));
    }
}

#[test]
fn a_config_that_turns_the_sandbox_off_keeps_no_boxes() {
    let mut config = config_with_backend(crate::config::SandboxBackend::OpenShell);
    config.sandbox.enabled = Some(false);
    assert!(!config_keeps_boxes(&config));
}
