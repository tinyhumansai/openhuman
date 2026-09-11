use super::*;

#[test]
fn shell_detects_python_runtime_commands() {
    for command in [
        "python3 -m pyfiglet hello",
        "python -m pip install pyfiglet",
        "pip install pyfiglet",
        "pip3.13 show pyfiglet",
        "/opt/openhuman/python/bin/python3 script.py",
        "echo hi && python3 -V",
    ] {
        assert!(
            shell_command_needs_python_runtime(command),
            "expected python runtime detection for {command}"
        );
    }

    for command in [
        "echo python3",
        "ls",
        "cat ./pipelines.txt",
        "node script.js",
    ] {
        assert!(
            !shell_command_needs_python_runtime(command),
            "did not expect python runtime detection for {command}"
        );
    }
}

#[test]
fn shell_runtime_path_prepends_managed_dirs_before_host_path() {
    let python = std::path::Path::new("/opt/openhuman/python/bin");
    let node = std::path::Path::new("/opt/openhuman/node/bin");
    let joined = prepend_path_dirs([python, node], "/usr/local/bin:/usr/bin");
    let sep = if cfg!(windows) { ";" } else { ":" };
    assert_eq!(
        joined,
        format!(
            "{}{}{}{}{}",
            python.display(),
            sep,
            node.display(),
            sep,
            "/usr/local/bin:/usr/bin"
        )
    );
}

/// Empirical answer to "does `shell` resolve/install managed Node on its
/// own?" — NO. The shell path consults the managed Node bootstrap only via
/// `try_cached()`, which never calls `resolve()` and therefore never
/// downloads/installs anything. So without a prior `node_exec` / `npm_exec`
/// (the tools that DO call `resolve()` and share this bootstrap instance),
/// `runtime_path_for_command` injects nothing for a node command. On a host
/// with no Node in the login PATH, the command then fails — the managed
/// runtime is never reached on the shell path. (Python, by contrast, IS
/// self-resolved in `runtime_path_for_command` — see the python branch.)
#[tokio::test]
async fn shell_does_not_resolve_or_install_node_on_its_own() {
    let node = Arc::new(NodeBootstrap::new(Arc::new(
        crate::openhuman::config::Config::default(),
    )));
    let tool = ShellTool::with_language_bootstraps(
        test_security(AutonomyLevel::Full),
        test_runtime(),
        test_audit(),
        Some(node),
        None,
    );

    // Unprimed (no prior node_exec/npm_exec resolve): shell injects NO
    // managed node bin onto PATH — it does not auto-resolve or install.
    let injected = tool
        .runtime_path_for_command("node --version")
        .await
        .expect("runtime path resolves");
    assert!(
        injected.is_none(),
        "shell injected a managed node bin without any prior node_exec/npm_exec \
         resolve — it must not auto-resolve/install on the shell path: {injected:?}"
    );
}

#[tokio::test]
async fn shell_blocks_rate_limited() {
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        max_actions_per_hour: 0,
        workspace_dir: std::env::temp_dir(),
        action_dir: std::env::temp_dir(),
        ..SecurityPolicy::default()
    });
    let tool = ShellTool::new(security, test_runtime(), test_audit());
    let result = tool.execute(json!({"command": "echo test"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Rate limit"));
}

#[cfg(not(windows))]
#[tokio::test]
async fn shell_sandboxed_mode_routes_through_sandbox_backend() {
    use crate::openhuman::agent::harness::definition::SandboxMode;
    use crate::openhuman::agent::harness::with_current_sandbox_mode;

    let tool = ShellTool::new(
        test_security(AutonomyLevel::Supervised),
        test_runtime(),
        test_audit(),
    );
    let result = with_current_sandbox_mode(SandboxMode::Sandboxed, async {
        tool.execute(json!({"command": "echo sandboxed-output"}))
            .await
            .unwrap()
    })
    .await;
    assert!(
        !result.is_error,
        "sandboxed echo should succeed: {}",
        result.output()
    );
    assert!(
        result.output().contains("sandboxed-output"),
        "expected 'sandboxed-output' in result, got: {:?}",
        result.output()
    );
}

/// Regression guard for #3235 (cwd_jail wiring for shell-family tools).
///
/// PR #3261 wired `ShellTool` to route through `sandbox::execute_in_sandbox`
/// (which uses `cwd_jail` for the local-OS-jail backend) when the
/// active agent's `SandboxMode::Sandboxed` is set. This PR extends the
/// same wiring to `NodeExecTool` and `NpmExecTool`. The behavioural
/// `shell_sandboxed_mode_routes_through_sandbox_backend` test above
/// proves the contract end-to-end for `shell` (no managed-Node
/// dependency); `node_exec` and `npm_exec` cannot run end-to-end in
/// unit tests without a resolved `NodeBootstrap`, so this source-grep
/// guard catches refactors that drop the sandbox check from either
/// tool's `execute()` body.
#[test]
fn shell_family_tools_route_to_sandbox_when_sandboxed_mode_active() {
    const SHELL_SRC: &str = include_str!("shell.rs");
    const NODE_EXEC_SRC: &str = include_str!("node_exec.rs");
    const NPM_EXEC_SRC: &str = include_str!("npm_exec.rs");

    for (name, src) in [
        ("shell.rs", SHELL_SRC),
        ("node_exec.rs", NODE_EXEC_SRC),
        ("npm_exec.rs", NPM_EXEC_SRC),
    ] {
        assert!(
            src.contains("current_sandbox_mode()"),
            "{name} must check `current_sandbox_mode()` to detect SandboxMode::Sandboxed \
             sessions and route through the sandbox backend (see #3235)"
        );
        assert!(
            src.contains("SandboxMode::Sandboxed"),
            "{name} must compare against `SandboxMode::Sandboxed` to opt in to the \
             sandbox routing path (see #3235)"
        );
        // Use the call-site pattern `.run_sandboxed(` so the assertion
        // doesn't trivially pass on the helper definition itself
        // (`fn run_sandboxed(...)`). If `execute()` / `run_with_security()`
        // stop delegating, this fires even though the helper still exists.
        assert!(
            src.contains(".run_sandboxed("),
            "{name} must delegate to a `run_sandboxed` helper when the sandbox mode is \
             active (see #3235). Whitespace before `.run_sandboxed` is tolerated; the \
             helper call must appear in the source — *not* just the helper definition."
        );
    }
}
