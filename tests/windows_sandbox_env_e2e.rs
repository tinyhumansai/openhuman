//! Windows child-environment e2e for OpenHuman's own sandbox execution path.
//!
//! Both `sandbox::ops` exec functions call `Command::env_clear()` and re-forward
//! only `SANDBOX_ENV_PASSTHROUGH`, so that list is the entire environment a
//! sandboxed child gets. This test measures what a real child sees when spawned
//! through `execute_in_sandbox` — the same function the `shell`, `node_exec`,
//! `npm_exec` and `python_exec` tools route into whenever the active agent is
//! `SandboxMode::Sandboxed` (which the built-in `orchestrator` is).
//!
//! The defect this pins: the list carried no Windows process-bootstrap
//! variables. A child without `SystemRoot` cannot initialise the OS crypto
//! provider, and the failures are opaque to the agent rather than diagnostic:
//!
//! - `node.exe` aborts at startup — `Assertion failed: ncrypto::CSPRNG(nullptr, 0)`, exit 134.
//! - `powershell.exe` exits with `Internal Windows PowerShell error. Loading
//!   managed Windows PowerShell failed with error 8009001d.`
//!
//! Note on method: these must be spawned from Rust. `node`'s own
//! `child_process.spawn` (libuv) injects `SystemRoot` into the child, and a
//! probe driven through `cmd.exe` from JavaScript reports it as present, so a
//! JS harness cannot observe the stripping at all.
//!
//! The Windows CI lane ignores this root integration target because it only
//! builds the core crate. Building the CLI integration target pulls in the full
//! CLI test graph; the lane documents that compiling every integration-test
//! binary can exhaust the Windows runner's paging file. Run it explicitly on
//! Windows with `cargo test -p openhuman-cli --test windows_sandbox_env_e2e -- --ignored`.

#[cfg(windows)]
use std::collections::HashMap;
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use openhuman_core::config::RuntimeConfig;
#[cfg(windows)]
use openhuman_core::sandbox::ops::{execute_in_sandbox, resolve_sandbox_policy};
#[cfg(windows)]
use tinyagents_definition::SandboxMode;

/// Run `command` through OpenHuman's sandbox execution path under `mode`.
#[cfg(windows)]
async fn run_in_sandbox(
    mode: SandboxMode,
    command: &str,
) -> (
    openhuman_core::sandbox::types::SandboxExecResult,
    std::path::PathBuf,
) {
    run_in_sandbox_with_env(mode, command, HashMap::new()).await
}

#[cfg(windows)]
async fn run_in_sandbox_with_env(
    mode: SandboxMode,
    command: &str,
    extra_env: HashMap<String, String>,
) -> (
    openhuman_core::sandbox::types::SandboxExecResult,
    std::path::PathBuf,
) {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().to_path_buf();
    let policy = resolve_sandbox_policy(
        mode,
        tempdir.path(),
        tempdir.path(),
        &RuntimeConfig::default(),
        false,
    );
    let result = execute_in_sandbox(
        &policy,
        command,
        tempdir.path(),
        extra_env
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect(),
        Duration::from_secs(120),
    )
    .await
    .unwrap_or_else(|e| panic!("execute_in_sandbox({mode:?}) failed to run the command: {e}"));
    (result, root)
}

#[cfg(windows)]
fn env_probe_command() -> &'static str {
    "echo SR=[%SystemRoot%] TEMP=[%TEMP%] UP=[%USERPROFILE%] PATH=[%PATH%]"
}

/// `cmd.exe` prints `%SystemRoot%` *verbatim* when the variable is absent, which
/// makes non-expansion the assertion. A resolved value is also required, so a
/// child that echoes nothing cannot pass.
#[cfg(windows)]
fn assert_bootstrap_env_resolved(result: &openhuman_core::sandbox::types::SandboxExecResult) {
    assert!(
        result.success(),
        "child exited {} with stderr {:?}",
        result.exit_code,
        result.stderr
    );
    for name in ["SystemRoot", "TEMP", "USERPROFILE", "PATH"] {
        assert!(
            !result.stdout.contains(&format!("%{name}%")),
            "`{name}` reached the child unexpanded, so it is not in the \
             forwarded environment; stdout {:?}",
            result.stdout
        );
    }
    let system_root = bracketed(&result.stdout, "SR=").unwrap_or_default();
    assert!(
        system_root.contains(':'),
        "`SystemRoot` did not resolve to a directory; stdout {:?}",
        result.stdout
    );
    assert!(
        !bracketed(&result.stdout, "UP=")
            .unwrap_or_default()
            .is_empty(),
        "`USERPROFILE` resolved to nothing; stdout {:?}",
        result.stdout
    );
    for name in ["TEMP", "PATH"] {
        assert!(
            !bracketed(&result.stdout, &format!("{name}="))
                .unwrap_or_default()
                .is_empty(),
            "`{name}` resolved to nothing; stdout {:?}",
            result.stdout
        );
    }
}

#[cfg(windows)]
fn assert_temp_is_in_scratch(
    result: &openhuman_core::sandbox::types::SandboxExecResult,
    root: &std::path::Path,
) {
    let temp = bracketed(&result.stdout, "TEMP=").unwrap_or_default();
    let expected_prefix = root.join("artifacts").join("sandbox-scratch");
    let normalize = |path: &str| path.replace('/', "\\").to_ascii_lowercase();
    assert!(
        normalize(&temp).starts_with(&normalize(&expected_prefix.to_string_lossy())),
        "local-jail TEMP escaped its per-test scratch tree: TEMP={temp:?}, expected under {:?}; stdout {:?}",
        expected_prefix,
        result.stdout
    );
}

#[cfg(windows)]
fn bracketed(stdout: &str, prefix: &str) -> Option<String> {
    let tail = stdout.split(prefix).nth(1)?;
    Some(tail.split(']').next()?.trim().to_string())
}

/// The unsandboxed exec path (`SandboxMode::None`), which shares the
/// allow-list with the jail path and has no OS-jail dependency.
#[cfg(windows)]
#[tokio::test]
#[ignore = "requires Windows and an explicit CLI integration-test build"]
async fn unsandboxed_child_receives_windows_bootstrap_env() {
    let (result, _) = run_in_sandbox(SandboxMode::None, env_probe_command()).await;
    assert_bootstrap_env_resolved(&result);
}

/// The route the built-in orchestrator actually takes:
/// `SandboxMode::Sandboxed` → `SandboxBackendKind::Local` →
/// `execute_local_jail`.
#[cfg(windows)]
#[tokio::test]
#[ignore = "requires Windows and an explicit CLI integration-test build"]
async fn sandboxed_child_receives_windows_bootstrap_env() {
    let (result, root) = run_in_sandbox(SandboxMode::Sandboxed, env_probe_command()).await;
    assert_bootstrap_env_resolved(&result);
    assert_temp_is_in_scratch(&result, &root);
}

/// Caller-provided temporary-directory values remain effective on both host
/// spawn paths; the local jail must not silently replace per-call overrides.
#[cfg(windows)]
#[tokio::test]
#[ignore = "requires Windows and an explicit CLI integration-test build"]
async fn caller_temp_overrides_survive_sandbox_paths() {
    let expected = r"C:\openhuman-test-temp";
    for mode in [SandboxMode::None, SandboxMode::Sandboxed] {
        let (result, _) = run_in_sandbox_with_env(
            mode,
            "echo TEMP=[%TEMP%] TMP=[%TMP%] TMPDIR=[%TMPDIR%]",
            HashMap::from([
                ("TEMP".to_string(), expected.to_string()),
                ("TMP".to_string(), expected.to_string()),
                ("TMPDIR".to_string(), expected.to_string()),
            ]),
        )
        .await;
        assert!(result.success(), "child failed: {:?}", result.stderr);
        for name in ["TEMP", "TMP", "TMPDIR"] {
            assert_eq!(
                bracketed(&result.stdout, &format!("{name}=")).as_deref(),
                Some(expected),
                "caller override for {name} was replaced; stdout {:?}",
                result.stdout
            );
        }
    }
}

/// The reported Node failure, end to end through OpenHuman's spawn code:
/// `node -e` must reach the crypto provider and print random bytes.
#[cfg(windows)]
#[tokio::test]
#[ignore = "requires Windows and an explicit CLI integration-test build"]
async fn node_crypto_runs_through_sandbox_path() {
    assert!(
        tool_available("node"),
        "node is required for node_crypto_runs_through_sandbox_path; missing tooling must not silently pass"
    );

    let (result, _) = run_in_sandbox(
        SandboxMode::None,
        r#"node -e "console.log('SR=' + process.env.SystemRoot); console.log(require('crypto').randomBytes(8).toString('hex'))""#,
    )
    .await;

    assert!(
        !result.stderr.contains("ncrypto"),
        "node's OS random provider failed to initialise — the child \
         environment is missing Windows bootstrap variables; stderr {:?}",
        result.stderr
    );
    assert!(
        result.success(),
        "node aborted through the sandbox path (exit {}); stderr {:?}",
        result.exit_code,
        result.stderr
    );
    assert!(
        result
            .stdout
            .lines()
            .any(|line| line.trim().len() == 16
                && line.trim().chars().all(|c| c.is_ascii_hexdigit())),
        "no random bytes were printed; stdout {:?}",
        result.stdout
    );
}

/// The reported PowerShell failure: `8009001d` when the child has no system
/// directory to load its crypto provider from.
#[cfg(windows)]
#[tokio::test]
#[ignore = "requires Windows and an explicit CLI integration-test build"]
async fn powershell_runs_through_sandbox_path() {
    assert!(
        tool_available("powershell.exe"),
        "powershell.exe is required for powershell_runs_through_sandbox_path; missing tooling must not silently pass"
    );

    let (result, _) = run_in_sandbox(
        SandboxMode::None,
        "powershell.exe -NoProfile -Command [guid]::NewGuid().ToString()",
    )
    .await;

    assert!(
        !result.stderr.contains("8009001d"),
        "PowerShell could not load managed PowerShell — the child environment \
         is missing Windows bootstrap variables; stderr {:?}",
        result.stderr
    );
    assert!(
        result.success(),
        "powershell failed through the sandbox path (exit {}); stderr {:?}",
        result.exit_code,
        result.stderr
    );
    let guid: String = result
        .stdout
        .trim()
        .chars()
        .filter(|c| c.is_ascii_hexdigit() || *c == '-')
        .collect();
    assert!(
        guid.len() >= 36,
        "no GUID was produced by the child; stdout {:?}",
        result.stdout
    );
}

#[cfg(windows)]
fn tool_available(program: &str) -> bool {
    let args = if program.eq_ignore_ascii_case("powershell.exe") {
        vec![
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$PSVersionTable.PSVersion.ToString()",
        ]
    } else {
        vec!["--version"]
    };
    std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
