//! Request mapping for the module's native shell member.

use super::*;

fn run_in(cwd: &std::path::Path, state: &std::path::Path) -> NativeShellRun {
    NativeShellRun::new("printf 'one two'", cwd, state, Duration::from_secs(7))
}

#[cfg(feature = "modules")]
#[test]
fn shell_exec_request_delegates_shell_selection_to_the_module() -> anyhow::Result<()> {
    let resource = contract::ResourceId("known-resource".to_owned());
    let process = contract::ResourceId("known-process".to_owned());
    let cwd = tempfile::tempdir()?;
    let state = tempfile::tempdir()?;
    let mut run = run_in(cwd.path(), state.path());
    run.env = BTreeMap::from([("LANG".to_owned(), "C".to_owned())]);
    run.preserve_pipeline_status = true;

    let request = shell_exec_request(process.clone(), &resource, run)?;

    assert_eq!(request.process, process);
    assert_eq!(request.resource, resource);
    assert_eq!(request.shell.command, "printf 'one two'");
    assert!(request.shell.preserve_pipeline_status);
    assert_eq!(request.shell.runtime_path, None);
    assert!(!request.shell.python_utf8);
    assert!(!request.hide_window);
    assert_eq!(request.cwd.as_deref(), cwd.path().to_str());
    assert_eq!(
        request.extra_env,
        BTreeMap::from([("LANG".to_owned(), "C".to_owned())])
    );
    assert_eq!(request.state_dir, state.path().to_str().unwrap());
    assert_eq!(request.timeout_ms, 7_000);
    assert_eq!(request.stdin, None);
    assert!(request.jail.is_none());
    Ok(())
}

#[cfg(feature = "modules")]
#[test]
fn shell_exec_request_forwards_python_utf8_and_window_suppression() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let mut run = run_in(dir.path(), dir.path());
    run.python_utf8 = true;
    run.hide_window = true;
    let request = shell_exec_request(
        contract::ResourceId("p".to_owned()),
        &contract::ResourceId("r".to_owned()),
        run,
    )?;
    assert!(request.shell.python_utf8);
    assert!(request.hide_window);
    Ok(())
}

/// The module rejects a budget over twenty-four hours, so an "unbounded" caller
/// gets the ceiling rather than a refused request; zero still runs for 1 ms.
#[cfg(feature = "modules")]
#[test]
fn shell_exec_request_keeps_the_runtime_budget_inside_the_contract_bounds() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let mut budgets = Vec::new();
    for timeout in [Duration::ZERO, Duration::from_secs(10 * 86_400)] {
        let mut run = run_in(dir.path(), dir.path());
        run.timeout = timeout;
        let request = shell_exec_request(
            contract::ResourceId("p".to_owned()),
            &contract::ResourceId("r".to_owned()),
            run,
        )?;
        budgets.push(request.timeout_ms);
    }
    assert_eq!(budgets, [1, 86_400_000]);
    Ok(())
}

#[cfg(feature = "modules")]
#[test]
fn a_non_utf8_working_directory_is_refused_before_the_bus() {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bad = std::path::Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff"));
        let state = std::path::Path::new("/tmp");
        let mut run = run_in(state, state);
        run.cwd = bad.to_path_buf();
        assert!(shell_exec_request(
            contract::ResourceId("p".to_owned()),
            &contract::ResourceId("r".to_owned()),
            run,
        )
        .is_err());
    }
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn a_disabled_module_never_falls_back_to_a_host_shell_for_a_jailed_run() {
    // The unjailed test fixture must not weaken the jail's refusal.
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("must-not-run");
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;
    let mut shell = NativeShellRun::new(
        &format!("touch '{}'", marker.display()),
        dir.path(),
        dir.path(),
        Duration::from_secs(5),
    );
    shell.jail = Some(contract::NativeJailConfig {
        read_only: Vec::new(),
        read_write: Vec::new(),
        allow_net: false,
        allow_subprocess: true,
        enforcement: contract::JailEnforcement::Enforced,
        require_process_isolation: true,
        deny_read: Vec::new(),
        resources: None,
    });
    assert!(run(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        shell
    )
    .await
    .is_err());
    assert!(!marker.exists());
}

/// The progress meter rides the run itself (no ambient scope), so output of a
/// command started through `run_host` reaches the meter its caller passed.
#[cfg(unix)]
#[tokio::test]
async fn run_host_feeds_the_callers_meter() {
    let dir = tempfile::tempdir().unwrap();
    let meter = crate::sandbox::shell_progress::ShellOutputMeter::default();
    let output = run_host(
        &crate::core::runtime::DispatchCtx::host_default(),
        &Config::default(),
        "printf 'line one\\nline two\\n'",
        dir.path(),
        dir.path(),
        BTreeMap::new(),
        Duration::from_secs(10),
        false,
        Some(meter.clone()),
    )
    .await
    .unwrap();
    assert!(output.success());
    assert_eq!(meter.snapshot(), (18, Some("line two".to_string())));
}

#[cfg(feature = "modules")]
#[test]
fn a_tinybox_timeout_is_the_commands_timeout_and_other_failures_name_their_code() {
    use contract::ExecutionFailure;
    assert!(failure_timed_out(ExecutionFailure::TimedOut).unwrap());
    let message = |code| failure_timed_out(code).unwrap_err().to_string();
    assert!(message(ExecutionFailure::OutputLimit).contains("output limit"));
    assert!(message(ExecutionFailure::StartFailed).contains("could not start"));
    assert!(message(ExecutionFailure::BackendFailed).contains("BackendFailed"));
    assert!(message(ExecutionFailure::CleanupFailed).contains("retained"));
}
