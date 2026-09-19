use super::*;
fn absolute_sample() -> &'static str {
    if cfg!(windows) {
        "C:\\Windows\\System32"
    } else {
        "/etc"
    }
}

#[test]
fn npm_timeout_policy_unbounded_by_default() {
    assert_eq!(npm_timeout_policy(&json!({})), ToolTimeout::Unbounded);
    assert_eq!(
        npm_timeout_policy(&json!({"timeout_secs": 0})),
        ToolTimeout::Unbounded
    );
}

#[test]
fn npm_timeout_policy_enforces_and_caps_explicit() {
    assert_eq!(
        npm_timeout_policy(&json!({"timeout_secs": 300})),
        ToolTimeout::Millis(300_000)
    );
    assert_eq!(
        npm_timeout_policy(&json!({"timeout_secs": 99999})),
        ToolTimeout::Millis(NPM_TIMEOUT_MAX_SECS * 1000)
    );
}

#[test]
fn is_sane_subcommand_accepts_common_npm_verbs() {
    for v in &[
        "install",
        "ci",
        "run",
        "exec",
        "test",
        "test:watch",
        "run-script",
    ] {
        assert!(is_sane_subcommand(v), "{v} should be accepted");
    }
}

#[test]
fn is_sane_subcommand_rejects_metacharacters() {
    for v in &["install; rm -rf /", "run && echo", "|cat", "$(whoami)", ""] {
        assert!(!is_sane_subcommand(v), "{v} should be rejected");
    }
}

#[test]
fn resolve_cwd_defaults_to_workspace() {
    let ws = std::path::Path::new("/tmp/ws");
    assert_eq!(resolve_cwd(ws, None).unwrap(), ws);
    assert_eq!(resolve_cwd(ws, Some("")).unwrap(), ws);
    assert_eq!(resolve_cwd(ws, Some(".")).unwrap(), ws);
}

#[test]
fn resolve_cwd_rejects_absolute_and_parent() {
    let ws = std::path::Path::new("/tmp/ws");
    assert!(resolve_cwd(ws, Some(absolute_sample())).is_err());
    assert!(resolve_cwd(ws, Some("../other")).is_err());
    assert!(resolve_cwd(ws, Some("sub/../../../etc")).is_err());
}

#[test]
fn resolve_cwd_allows_relative_subdir() {
    let ws = std::path::Path::new("/tmp/ws");
    let got = resolve_cwd(ws, Some("app")).unwrap();
    assert_eq!(got, std::path::PathBuf::from("/tmp/ws/app"));
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
