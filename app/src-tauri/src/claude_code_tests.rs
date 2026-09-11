use super::*;

/// Every launcher must reach the `auth` subcommand. `claude login` is not a
/// command — `claude` takes a positional `[prompt]`, so it silently became a
/// prompt instead of starting the OAuth flow.
fn assert_reaches_auth_login(rendered: &str, what: &str) {
    assert!(
        rendered.contains("claude auth login"),
        "{what} must invoke `claude auth login`, got: {rendered}"
    );
    assert!(
        !rendered.contains("claude login"),
        "{what} still constructs the obsolete `claude login`, got: {rendered}"
    );
}

#[test]
fn login_command_line_is_the_auth_subcommand() {
    assert_eq!(login_command_line(), "claude auth login --claudeai");
}

#[test]
fn argv_keeps_auth_and_login_as_separate_words() {
    // Split-argument terminals pass these straight to execvp, so `auth` and
    // `login` have to be distinct argv entries rather than one "auth login".
    assert_eq!(CLAUDE_LOGIN_ARGV, &["claude", "auth", "login", "--claudeai"]);
}

#[test]
fn windows_launcher_reaches_auth_login() {
    assert_reaches_auth_login(&windows_launch_args().join(" "), "the Windows launcher");
}

#[test]
fn windows_launcher_keeps_the_empty_start_title() {
    // `start` reads a bare first argument as the window title, which would
    // swallow `cmd` and open an empty shell.
    let args = windows_launch_args();
    assert_eq!(args[1], "start");
    assert_eq!(args[2], "", "the empty title placeholder must survive");
}

#[test]
fn macos_launcher_reaches_auth_login() {
    assert_reaches_auth_login(&macos_launch_script(), "the macOS AppleScript");
}

#[test]
fn macos_script_quotes_the_command_for_do_script() {
    assert!(macos_launch_script().contains(r#"do script "claude auth login --claudeai""#));
}

#[test]
fn every_linux_candidate_reaches_auth_login() {
    let candidates = linux_launch_candidates();
    assert_eq!(candidates.len(), 5, "all five emulators stay covered");
    for (term, args) in candidates {
        assert_reaches_auth_login(&args.join(" "), term);
    }
}

#[test]
fn xfce4_terminal_gets_one_string_and_the_others_get_argv() {
    let candidates = linux_launch_candidates();
    for (term, args) in candidates {
        match term {
            // -e plus a single command string
            "xfce4-terminal" => assert_eq!(args.len(), 2, "xfce4-terminal takes one string"),
            // separator plus four argv words
            _ => assert_eq!(args.len(), 5, "{term} takes the command as argv"),
        }
    }
}

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
