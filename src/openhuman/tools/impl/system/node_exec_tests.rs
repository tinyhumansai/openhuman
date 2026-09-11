use super::*;
fn absolute_sample() -> &'static str {
    if cfg!(windows) {
        "C:\\Windows\\System32\\drivers\\etc\\hosts"
    } else {
        "/etc/passwd"
    }
}

#[test]
fn shell_quote_wraps_plain_strings() {
    assert_eq!(shell_quote("node"), "'node'");
    assert_eq!(shell_quote("/opt/bin/node"), "'/opt/bin/node'");
}

#[test]
fn node_timeout_policy_unbounded_by_default() {
    // No timeout_secs (or explicit 0) ⇒ run to completion.
    assert_eq!(node_timeout_policy(&json!({})), ToolTimeout::Unbounded);
    assert_eq!(
        node_timeout_policy(&json!({"timeout_secs": 0})),
        ToolTimeout::Unbounded
    );
}

#[test]
fn node_timeout_policy_enforces_and_caps_explicit() {
    assert_eq!(
        node_timeout_policy(&json!({"timeout_secs": 120})),
        ToolTimeout::Secs(120)
    );
    // Clamped to the 1800s ceiling.
    assert_eq!(
        node_timeout_policy(&json!({"timeout_secs": 99999})),
        ToolTimeout::Secs(NODE_TIMEOUT_MAX_SECS)
    );
}

#[test]
fn process_chdir_snippets_use_legacy_node_spawn() {
    for code in [
        "process.chdir('subdir'); console.log(process.cwd())",
        "const move = process.chdir; move('subdir')",
        "const { chdir } = process; chdir('subdir')",
        "process['chdir']('subdir')",
    ] {
        assert!(
            inline_requires_process_chdir_compat(code),
            "expected legacy fallback for {code:?}"
        );
    }
    assert!(!inline_requires_process_chdir_compat(
        "console.log(process.cwd())"
    ));
}

#[test]
fn shell_quote_escapes_single_quotes() {
    assert_eq!(shell_quote("it's"), "'it'\\''s'");
    assert_eq!(
        shell_quote("console.log('hi')"),
        "'console.log('\\''hi'\\'')'"
    );
}

#[test]
fn shell_quote_neutralises_metacharacters() {
    // $, backticks, && — all inert once wrapped in single quotes.
    assert_eq!(shell_quote("$(rm -rf /)"), "'$(rm -rf /)'");
    assert_eq!(shell_quote("a && b"), "'a && b'");
}

#[test]
fn resolve_script_path_rejects_empty() {
    let ws = std::path::Path::new("/ws");
    assert!(resolve_script_path(ws, "").is_err());
    assert!(resolve_script_path(ws, "   ").is_err());
}

#[test]
fn resolve_script_path_rejects_absolute() {
    let ws = std::path::Path::new("/ws");
    assert!(resolve_script_path(ws, absolute_sample()).is_err());
}

#[test]
fn resolve_script_path_rejects_parent_dir() {
    let ws = std::path::Path::new("/ws");
    assert!(resolve_script_path(ws, "../evil.js").is_err());
    assert!(resolve_script_path(ws, "scripts/../../evil.js").is_err());
}

#[test]
fn resolve_script_path_accepts_relative_subdir() {
    let ws = std::path::Path::new("/ws");
    let resolved = resolve_script_path(ws, "scripts/run.js").unwrap();
    assert_eq!(resolved, std::path::Path::new("/ws/scripts/run.js"));
}

#[test]
fn safe_env_vars_include_windows_process_essentials() {
    for var in ["SystemRoot", "COMSPEC", "PATHEXT", "TEMP", "USERPROFILE"] {
        assert!(
            SAFE_ENV_VARS.contains(&var),
            "{var} must be forwarded for Windows child processes"
        );
    }
}

/// Regression guard for #3238.
///
/// `node_exec` resolves caller-supplied `script_path` values against
/// `security.action_dir` (the agent's writable sandbox), never
/// `security.workspace_dir` (internal product state). If a future
/// refactor changes `NodeExecTool::execute` to pass
/// `&self.security.workspace_dir` to `resolve_script_path`, scripts
/// would resolve into the internal denylist instead of the action
/// sandbox, which is exactly the action/internal split that
/// PR #3074 prevents.
///
/// The behavioural end-to-end test for the CWD plumbing lives in
/// `shell.rs` (`shell_pwd_returns_action_dir_not_workspace_dir`) —
/// `node_exec` shares the same `runtime.build_shell_command(&command,
/// &self.security.action_dir)` call site, and the source-grep guard
/// in `shell.rs` (`shell_family_tools_route_cwd_through_action_dir`)
/// covers all three system tools. This test pins the script-resolution
/// contract specifically for `node_exec` by exercising
/// `resolve_script_path` against an `action_dir` distinct from
/// `workspace_dir`.
#[test]
fn resolve_script_path_targets_action_dir_not_workspace_dir() {
    let action_dir = std::path::Path::new("/tmp/action-sandbox-3238");
    let workspace_dir = std::path::Path::new("/tmp/internal-workspace-3238");

    let resolved = resolve_script_path(action_dir, "scripts/run.js")
        .expect("relative script under action_dir must resolve");
    assert_eq!(
        resolved,
        action_dir.join("scripts/run.js"),
        "script_path must resolve under action_dir, not workspace_dir (see #3238)"
    );
    assert!(
        resolved.starts_with(action_dir),
        "resolved path must be under action_dir; got {}",
        resolved.display()
    );
    assert!(
        !resolved.starts_with(workspace_dir),
        "resolved path leaked into workspace_dir; got {}",
        resolved.display()
    );
}

#[tokio::test]
async fn inline_code_cannot_write_to_sibling_profile() {
    use crate::openhuman::agent::host_runtime::NativeRuntime;
    use crate::openhuman::security::policy::ActiveProfileGuard;
    use crate::openhuman::security::AutonomyLevel;

    let temp = tempfile::tempdir().unwrap();
    let action_root = temp.path().join("actions");
    let alice = action_root.join("profiles/alice");
    std::fs::create_dir_all(action_root.join("profiles/bob")).unwrap();
    std::fs::create_dir_all(&alice).unwrap();
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_dir: temp.path().join("state"),
        action_dir: alice,
        workspace_only: false,
        active_profile: Some(ActiveProfileGuard {
            profile_id: "alice".into(),
            action_dir: action_root,
        }),
        ..SecurityPolicy::default()
    });
    let bootstrap = Arc::new(NodeBootstrap::new(Arc::new(
        crate::openhuman::config::Config::default(),
    )));
    let tool = NodeExecTool::new(
        security,
        Arc::new(NativeRuntime::new()),
        bootstrap,
        crate::openhuman::config::RuntimePoolConfig::default(),
        temp.path().join("state"),
    );

    let result = tool
        .execute(json!({
            "inline_code": "require('fs').writeFileSync('../bob/loot.txt', 'x')"
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.text().contains("Cross-profile access blocked"));
}
