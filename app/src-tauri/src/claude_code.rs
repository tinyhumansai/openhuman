//! Tauri commands for the Claude Code CLI provider.
//!
//! Provides a cross-platform "open a terminal and run `claude auth login`"
//! helper. The CLI's OAuth flow is interactive (it prints a URL and
//! waits for the user to paste a code), so we can't host it in-app — we
//! detach into the user's native terminal so they complete login there,
//! then return to OpenHuman and click Recheck in the settings card.

#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::process::Command;

/// The CLI's authentication entry point, as argv.
///
/// Authentication lives under the `auth` subcommand: `claude auth login`.
/// There is no top-level `login` command, and because `claude` accepts a
/// positional `[prompt]`, the old `claude login` was not rejected — it was
/// read as a prompt and started an ordinary session, so the sign-in flow
/// never ran and the settings card kept reporting the account as signed out.
///
/// `--claudeai` is the CLI's own default; it is passed explicitly so the
/// launcher does not silently follow a future change of that default into
/// console/API billing.
const CLAUDE_LOGIN_ARGV: &[&str] = &["claude", "auth", "login", "--claudeai"];

/// The same command as one shell word, for terminals that take a string
/// rather than an argv (`cmd /k`, AppleScript `do script`, `xfce4-terminal -e`).
fn login_command_line() -> String {
    CLAUDE_LOGIN_ARGV.join(" ")
}

/// Argv for `cmd /c start "" cmd /k <login command>`.
/// Compiled when testing on any host too, so a Linux CI run still checks the
/// Windows argv; otherwise only where it is actually spawned. Same for the
/// macOS and Linux helpers below.
#[cfg(any(test, target_os = "windows"))]
fn windows_launch_args() -> Vec<String> {
    // `start ""` opens a new console window; the empty quoted title
    // prevents cmd from interpreting the first arg as a title.
    // `cmd /k` keeps the window open after the command exits so the
    // user can read any final output.
    vec![
        "/c".into(),
        "start".into(),
        "".into(),
        "cmd".into(),
        "/k".into(),
        login_command_line(),
    ]
}

/// The AppleScript handed to `osascript`.
#[cfg(any(test, target_os = "macos"))]
fn macos_launch_script() -> String {
    format!(
        r#"tell application "Terminal"
    activate
    do script "{}"
end tell"#,
        login_command_line()
    )
}

/// Linux terminal emulators to try, in order, with the args each one needs.
#[cfg(any(test, target_os = "linux"))]
fn linux_launch_candidates() -> Vec<(&'static str, Vec<String>)> {
    let argv: Vec<String> = CLAUDE_LOGIN_ARGV.iter().map(|s| (*s).to_string()).collect();
    let line = login_command_line();
    vec![
        ("x-terminal-emulator", {
            let mut a = vec!["-e".to_string()];
            a.extend(argv.clone());
            a
        }),
        ("gnome-terminal", {
            let mut a = vec!["--".to_string()];
            a.extend(argv.clone());
            a
        }),
        ("konsole", {
            let mut a = vec!["-e".to_string()];
            a.extend(argv.clone());
            a
        }),
        // xfce4-terminal takes the command as a single string, not as argv.
        ("xfce4-terminal", vec!["-e".to_string(), line]),
        ("xterm", {
            let mut a = vec!["-e".to_string()];
            a.extend(argv);
            a
        }),
    ]
}

/// Open the user's native terminal and run `claude auth login` inside it.
///
/// Returns the name of the terminal emulator we launched (for UI
/// confirmation) or an error string if no terminal could be opened.
///
/// Platform behaviour:
///   - Windows: `cmd /c start "" cmd /k claude auth login --claudeai`
///   - macOS:   `osascript` → Terminal.app `do script "claude auth login --claudeai"`
///   - Linux:   try `x-terminal-emulator`, then `gnome-terminal`,
///              `konsole`, `xfce4-terminal`, `xterm` in that order
#[tauri::command]
pub async fn claude_code_login_launch() -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(windows_launch_args())
            .spawn()
            .map_err(|e| format!("failed to open cmd: {e}"))?;
        Ok("cmd".into())
    }

    #[cfg(target_os = "macos")]
    {
        // Starting osascript does not prove Terminal opened: Automation denial
        // is reported by its exit status. Await it off the UI thread and bound
        // the wait while macOS may be showing a permission prompt.
        let mut command = tokio::process::Command::new("/usr/bin/osascript");
        command.args(["-e", &macos_launch_script()]);
        wait_for_terminal(command, std::time::Duration::from_secs(30)).await?;
        Ok("Terminal.app".into())
    }

    #[cfg(target_os = "linux")]
    {
        for (term, args) in linux_launch_candidates() {
            match Command::new(term).args(&args).spawn() {
                Ok(_) => return Ok(term.to_string()),
                Err(_) => continue,
            }
        }
        Err(format!(
            "no terminal emulator found (tried x-terminal-emulator, gnome-terminal, konsole, \
             xfce4-terminal, xterm). Run `{}` manually.",
            login_command_line()
        ))
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Err("claude_code_login_launch is not supported on this platform".into())
    }
}

#[cfg(any(target_os = "macos", test))]
async fn wait_for_terminal(
    mut command: tokio::process::Command,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let output = tokio::time::timeout(timeout, command.kill_on_drop(true).output())
        .await
        .map_err(|_| {
            log::warn!("[claude-code][login] Terminal launch timed out");
            "Timed out opening Terminal.app".to_string()
        })?
        .map_err(|e| {
            log::warn!("[claude-code][login] Terminal launch spawn failed: {e}");
            format!("failed to open Terminal.app: {e}")
        })?;
    if !output.status.success() {
        log::warn!(
            "[claude-code][login] Terminal launch failed: {}",
            output.status
        );
        return Err(format!("Terminal.app launch failed: {}", output.status));
    }
    log::debug!("[claude-code][login] Terminal.app opened");
    Ok(())
}

#[cfg(test)]
#[path = "claude_code_tests.rs"]
mod tests;
