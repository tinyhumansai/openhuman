use super::*;

#[test]
fn python_timeout_policy_unbounded_by_default() {
    assert_eq!(python_timeout_policy(&json!({})), ToolTimeout::Unbounded);
    assert_eq!(
        python_timeout_policy(&json!({"timeout_secs": 0})),
        ToolTimeout::Unbounded
    );
}

#[test]
fn python_timeout_policy_enforces_and_caps_explicit() {
    assert_eq!(
        python_timeout_policy(&json!({"timeout_secs": 120})),
        ToolTimeout::Secs(120)
    );
    assert_eq!(
        python_timeout_policy(&json!({"timeout_secs": 99999})),
        ToolTimeout::Secs(PYTHON_TIMEOUT_MAX_SECS)
    );
}

#[test]
fn shell_quote_escapes_single_quotes() {
    assert_eq!(shell_quote("it's"), "'it'\\''s'");
    assert_eq!(shell_quote("print('hi')"), "'print('\\''hi'\\'')'");
}

#[test]
fn resolve_script_path_rejects_escapes() {
    let ws = std::path::Path::new("/ws");
    assert!(resolve_script_path(ws, "").is_err());
    assert!(resolve_script_path(ws, "../evil.py").is_err());
    assert_eq!(
        resolve_script_path(ws, "scripts/run.py").unwrap(),
        std::path::Path::new("/ws/scripts/run.py")
    );
}
