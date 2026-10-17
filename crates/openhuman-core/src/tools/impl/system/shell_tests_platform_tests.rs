//! Platform-aware shell description (`shell_description.rs`) and the Python
//! encoding defaults, exercised through the `shell` tool.
//!
//! The description takes the platform as a parameter so the Windows wording is
//! pinned on every CI host, not only on a Windows runner.

use super::*;
use crate::agent::host_runtime::{host_shell_flavor, ShellFlavor};
#[cfg(feature = "modules")]
use crate::tools::system::shell_support::PYTHON_UTF8_DEFAULTS;
use std::collections::HashMap;
use std::ffi::OsString;

#[test]
fn windows_description_names_cmd_and_its_equivalents_of_posix_commands() {
    // Production (0.64.x): models sent `pwd; ls` and `cat` to cmd.exe because
    // the description read like a POSIX shell.
    let desc = shell_description(ShellFlavor::Cmd);
    assert!(desc.contains("cmd.exe"), "{desc}");
    for equivalent in ["`cd`", "`dir`", "`type`", "`findstr`", "`%VAR%`", "`&&`"] {
        assert!(desc.contains(equivalent), "missing {equivalent}: {desc}");
    }
    assert!(
        desc.contains("powershell"),
        "names the PowerShell escape hatch: {desc}"
    );
    // No POSIX app-launch examples on Windows.
    assert!(!desc.contains("xdg-open"), "{desc}");
    assert!(!desc.contains("open -a"), "{desc}");
}

#[test]
fn posix_description_does_not_mention_cmd() {
    let desc = shell_description(ShellFlavor::Posix);
    assert!(!desc.contains("cmd.exe"), "{desc}");
    assert!(desc.contains("print what you need"), "{desc}");
}

#[test]
fn every_description_keeps_the_stdout_only_warning() {
    for flavor in [ShellFlavor::Cmd, ShellFlavor::Posix] {
        assert!(
            shell_description(flavor).contains("Only stdout/stderr comes back"),
            "{flavor:?}"
        );
    }
}

#[test]
fn the_tool_describes_the_shell_this_host_actually_spawns() {
    let tool = ShellTool::new(
        test_security(AutonomyLevel::Supervised),
        test_runtime(),
        test_audit(),
    );
    assert_eq!(tool.description(), shell_description(host_shell_flavor()));
    let command_desc = tool.parameters_schema()["properties"]["command"]["description"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(command_desc, command_param_description(host_shell_flavor()));
    assert!(command_param_description(ShellFlavor::Cmd).contains("cmd.exe"));
}

fn parent(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let map: HashMap<String, OsString> = vars
        .iter()
        .map(|(k, v)| (k.to_string(), OsString::from(v)))
        .collect();
    move |name: &str| map.get(name).cloned()
}

fn as_map(env: Vec<(&'static str, OsString)>) -> HashMap<&'static str, String> {
    env.into_iter()
        .map(|(k, v)| (k, v.to_string_lossy().into_owned()))
        .collect()
}

#[test]
fn a_users_own_python_encoding_settings_win() {
    let env = as_map(python_utf8_env(parent(&[
        ("PYTHONUTF8", "0"),
        ("PYTHONIOENCODING", "latin-1"),
    ])));
    assert_eq!(env.get("PYTHONUTF8").map(String::as_str), Some("0"));
    assert_eq!(
        env.get("PYTHONIOENCODING").map(String::as_str),
        Some("latin-1")
    );
}

/// A Docker container gets the sandbox allowlist from the turn's environment
/// plus the Python defaults, and never the rest of it.
#[cfg(feature = "modules")]
#[tokio::test]
async fn the_container_environment_forwards_only_the_allowlist_plus_python_defaults() {
    let scoped = tinytools_std::timeout::CommandEnvironment::new([
        ("PATH".to_owned(), "/bin".to_owned()),
        ("OPENAI_API_KEY".to_owned(), "sk-secret".to_owned()),
        ("AWS_SECRET_ACCESS_KEY".to_owned(), "secret".to_owned()),
    ]);
    let env = scoped
        .scope(async { container_environment(None) })
        .await
        .unwrap();
    assert_eq!(env.get("PATH").map(String::as_str), Some("/bin"));
    assert_eq!(env.get("PYTHONUTF8").map(String::as_str), Some("1"));
    assert!(!env.contains_key("OPENAI_API_KEY"));
    assert!(!env.contains_key("AWS_SECRET_ACCESS_KEY"));
    for key in env.keys() {
        assert!(
            crate::sandbox::ops::SANDBOX_ENV_PASSTHROUGH.contains(&key.as_str())
                || PYTHON_UTF8_DEFAULTS.iter().any(|(k, _)| k == key),
            "unexpected {key}"
        );
    }
}

/// Explicit turn variables reach the module-backed shell without inheriting
/// unrelated host variables.
#[cfg(feature = "modules")]
#[test]
fn the_container_environment_forwards_explicit_turn_variables() {
    let explicit = std::collections::BTreeMap::from([
        ("TURN_OWNER".to_owned(), "isolated".to_owned()),
        ("WORKER_LABEL".to_owned(), "test".to_owned()),
    ]);
    let env = container_environment(Some(&explicit)).unwrap();
    assert_eq!(env.get("TURN_OWNER").map(String::as_str), Some("isolated"));
    assert_eq!(env.get("WORKER_LABEL").map(String::as_str), Some("test"));
    assert!(!env.contains_key("PATH"));
    assert_eq!(env.get("PYTHONUTF8").map(String::as_str), Some("1"));

    let mut sandbox = HashMap::<OsString, OsString>::new();
    super::super::super::shell_support::extend_explicit_environment(&mut sandbox, Some(&explicit));
    assert_eq!(
        sandbox.get(OsString::from("TURN_OWNER").as_os_str()),
        Some(&OsString::from("isolated"))
    );
}

#[test]
fn sandboxed_calls_get_the_same_python_defaults() {
    let env = as_map(python_utf8_env(parent(&[])));
    assert_eq!(env.get("PYTHONUTF8").map(String::as_str), Some("1"));
    assert_eq!(
        env.get("PYTHONIOENCODING").map(String::as_str),
        Some("utf-8")
    );
}

#[tokio::test]
async fn a_spawned_shell_sees_the_python_utf8_defaults() {
    // A developer's own setting wins, so expect it when present.
    let expected = std::env::var("PYTHONUTF8").unwrap_or_else(|_| "1".to_string());
    let tool = ShellTool::new(test_security_with_env_cmd(), test_runtime(), test_audit());
    let command = if cfg!(windows) {
        "echo %PYTHONUTF8%"
    } else {
        "echo $PYTHONUTF8"
    };
    let result = tool
        .execute(serde_json::json!({ "command": command }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    assert_eq!(result.output().trim(), expected);
}

/// A runtime that spawns cmd.exe regardless of the host, so the description
/// is provably taken from the runtime rather than from `cfg!(windows)`.
struct CmdRuntime;

impl RuntimeAdapter for CmdRuntime {
    fn name(&self) -> &str {
        "cmd-test"
    }
    fn has_shell_access(&self) -> bool {
        true
    }
    fn has_filesystem_access(&self) -> bool {
        true
    }
    fn storage_path(&self) -> std::path::PathBuf {
        std::env::temp_dir()
    }
    fn supports_long_running(&self) -> bool {
        true
    }
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Cmd
    }
}

#[test]
fn the_tool_describes_the_shell_its_runtime_spawns() {
    let cmd_tool = ShellTool::new(
        test_security(AutonomyLevel::Supervised),
        Arc::new(CmdRuntime),
        test_audit(),
    );
    assert_eq!(cmd_tool.description(), shell_description(ShellFlavor::Cmd));
    assert_eq!(
        cmd_tool.parameters_schema()["properties"]["command"]["description"],
        command_param_description(ShellFlavor::Cmd)
    );
}

#[test]
fn the_docker_runtime_always_runs_posix_sh() {
    let docker = crate::agent::host_runtime::create_runtime(
        &crate::config::RuntimeConfig {
            kind: "docker".into(),
            ..crate::config::RuntimeConfig::default()
        },
        false,
    )
    .expect("docker runtime");
    assert_eq!(docker.shell_flavor(), ShellFlavor::Posix);
}
