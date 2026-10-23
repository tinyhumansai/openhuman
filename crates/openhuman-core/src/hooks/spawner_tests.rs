//! Hook commands run behind the untrusted-spawn gate or not at all.

use super::*;
use std::collections::BTreeMap;
use std::path::Path;
use tinyagents_runtime::command_hooks::HookEnvironment;

fn config_in(root: &Path) -> Config {
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        config_path: root.join("config.toml"),
        ..Config::default()
    };
    config.modules.enabled = false;
    config
}

fn request(command: &str, stdin: &[u8]) -> CommandRequest {
    CommandRequest {
        command: command.to_owned(),
        cwd: None,
        env: BTreeMap::new(),
        stdin: stdin.to_vec(),
        timeout: Duration::from_secs(10),
    }
}

/// With no jail reachable, the hook command must be refused, not started on the
/// host, and the engine must see an error it can turn into a denial.
#[tokio::test]
async fn a_hook_command_never_runs_when_the_jail_is_unreachable() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(tmp.path());
    let marker = tmp.path().join("ran");

    let result = run_in(
        &config,
        request(&format!("touch '{}'", marker.display()), b"{}"),
    )
    .await;

    assert!(result.is_err(), "the jail is unreachable: {result:?}");
    assert!(!marker.exists(), "a refused spawn must never run the hook");
}

/// The operator's own opt-out is the only unjailed path; through it the hook
/// still gets its stdin, environment and exit code.
#[tokio::test]
async fn a_hook_command_gets_stdin_environment_and_exit_code_through_the_gate() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(tmp.path());
    config.security.sandbox.enabled = Some(false);
    let mut spawn = request("cat; echo \"$HOOK_VAR\" 1>&2; exit 2", br#"{"event":"x"}"#);
    spawn.env.insert("HOOK_VAR".into(), "from-engine".into());

    let output = run_in(&config, spawn).await.expect("the hook ran");

    assert_eq!(output.exit_code, Some(2));
    assert_eq!(output.stdout, br#"{"event":"x"}"#);
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        "from-engine"
    );
}

/// The engine, wired to this spawner, denies a `fail_closed` hook when the gate
/// refuses it, and never starts the process.
#[tokio::test]
async fn a_refused_hook_denies_when_it_is_fail_closed() {
    use tinyagents_runtime::command_hooks::config::{HookDefinition, HookKind, LoopLimit};
    use tinyagents_runtime::command_hooks::types::{HookInput, HookPayload, ShellPayload};

    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join("ran");
    let definition = HookDefinition {
        command: format!("touch '{}'", marker.display()),
        kind: HookKind::Command,
        timeout: Some(5),
        matcher: None,
        loop_limit: LoopLimit::Default,
        fail_closed: true,
        model: None,
        enabled: true,
        layer: None,
        source_dir: None,
    };
    let input = HookInput {
        hook_event_name: "preToolUse".into(),
        conversation_id: None,
        generation_id: None,
        session_id: None,
        model: None,
        agent_id: None,
        openhuman_version: "test".into(),
        workspace_roots: vec![],
        cwd: None,
        payload: HookPayload::Shell(ShellPayload {
            command: "ls".into(),
            sandbox: false,
            output: None,
            duration_ms: None,
        }),
    };
    let config = config_in(tmp.path());
    // The engine reaches the spawner the same way `host::environment` wires it.
    struct Fixed(Config);
    #[async_trait]
    impl CommandSpawner for Fixed {
        async fn run(&self, request: CommandRequest) -> Result<CommandOutput, String> {
            run_in(&self.0, request).await
        }
    }
    let environment = HookEnvironment::new("OpenHuman", None)
        .with_command_spawner(std::sync::Arc::new(Fixed(config)));

    let run = tinyagents_runtime::command_hooks::exec::run(
        &definition,
        &input,
        &BTreeMap::new(),
        Duration::from_secs(5),
        &environment,
    )
    .await;

    assert!(run.output.is_deny(), "{run:?}");
    assert!(!marker.exists());
}

#[test]
fn a_timeout_is_an_error_not_an_exit_code() {
    let timed_out = crate::sandbox::types::SandboxExecResult {
        exit_code: -1,
        stdout: String::new(),
        stderr: String::new(),
        timed_out: true,
    };
    let error = outcome(Ok(timed_out), Duration::from_secs(3)).unwrap_err();
    assert!(error.contains("timed out after 3s"), "{error}");
}

#[test]
fn the_isolation_floor_is_reported_as_a_security_block() {
    let floor = anyhow::Error::new(crate::sandbox::ops::IsolationFloor).context("jail below floor");
    let error = outcome(Err(floor), Duration::from_secs(3)).unwrap_err();
    assert!(error.starts_with("blocked by security policy"), "{error}");
}
