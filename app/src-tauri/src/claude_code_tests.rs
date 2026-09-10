use super::*;

#[test]
fn main_window_can_invoke_the_registered_login_command() {
    let capability: serde_json::Value =
        serde_json::from_str(include_str!("../capabilities/claude-code-login.json")).unwrap();
    assert_eq!(capability["windows"], serde_json::json!(["main"]));
    assert_eq!(
        capability["permissions"],
        serde_json::json!(["allow-claude-code-login"])
    );
    let permissions: toml::Value =
        toml::from_str(include_str!("../permissions/allow-claude-code-login.toml")).unwrap();
    let permission = &permissions["permission"][0];
    assert_eq!(
        permission["identifier"].as_str(),
        Some("allow-claude-code-login")
    );
    assert_eq!(
        permission["commands"]["allow"].as_array().unwrap(),
        &[toml::Value::String("claude_code_login_launch".into())]
    );
    assert!(permission["commands"]["deny"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(include_str!("lib.rs").contains("claude_code::claude_code_login_launch"));
}

#[cfg(unix)]
#[tokio::test]
async fn terminal_launcher_requires_a_successful_exit() {
    for (code, expected_ok) in [(0, true), (7, false)] {
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args(["-c", &format!("exit {code}")]);
        let result = wait_for_terminal(command, std::time::Duration::from_secs(5)).await;
        assert_eq!(result.is_ok(), expected_ok);
        if let Err(error) = result {
            assert!(error.contains("Terminal.app launch failed"));
        }
    }
}

#[tokio::test]
async fn missing_terminal_launcher_reports_spawn_failure() {
    let root = tempfile::tempdir().unwrap();
    let command = tokio::process::Command::new(root.path().join("missing-launcher"));
    let error = wait_for_terminal(command, std::time::Duration::from_secs(5))
        .await
        .unwrap_err();
    assert!(error.contains("failed to open Terminal.app"));
}

#[cfg(unix)]
#[tokio::test]
async fn stuck_terminal_launcher_is_bounded() {
    let mut command = tokio::process::Command::new("/bin/sleep");
    command.arg("60");
    let error = wait_for_terminal(command, std::time::Duration::from_millis(50))
        .await
        .unwrap_err();
    assert!(error.contains("Timed out"));
}
