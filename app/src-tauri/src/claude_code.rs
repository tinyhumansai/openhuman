//! Tauri commands for the Claude Code CLI provider.
//!
//! Provides a cross-platform "open a terminal and run `claude login`"
//! helper. The CLI's OAuth flow is interactive (it prints a URL and
//! waits for the user to paste a code), so we can't host it in-app — we
//! detach into the user's native terminal so they complete login there,
//! then return to OpenHuman and click Recheck in the settings card.

#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::process::Command;

/// Open the user's native terminal and run `claude login` inside it.
///
/// Returns the name of the terminal emulator we launched (for UI
/// confirmation) or an error string if no terminal could be opened.
///
/// Platform behaviour:
///   - Windows: `cmd /c start "" cmd /k claude login`
///   - macOS:   `osascript` → Terminal.app `do script "claude login"`
///   - Linux:   try `x-terminal-emulator`, then `gnome-terminal`,
///              `konsole`, `xterm` in that order
#[tauri::command]
pub async fn claude_code_login_launch() -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        // `start ""` opens a new console window; the empty quoted title
        // prevents cmd from interpreting the first arg as a title.
        // `cmd /k` keeps the window open after `claude login` exits so
        // the user can read any final output.
        Command::new("cmd")
            .args(["/c", "start", "", "cmd", "/k", "claude login"])
            .spawn()
            .map_err(|e| format!("failed to open cmd: {e}"))?;
        return Ok("cmd".into());
    }

    #[cfg(target_os = "macos")]
    {
        let script = r#"tell application "Terminal"
    activate
    do script "claude login"
end tell"#;
        // Starting osascript does not prove Terminal opened: Automation denial
        // is reported by its exit status. Await it off the UI thread and bound
        // the wait while macOS may be showing a permission prompt.
        let mut command = tokio::process::Command::new("/usr/bin/osascript");
        command.args(["-e", script]);
        wait_for_terminal(command, std::time::Duration::from_secs(30)).await?;
        Ok("Terminal.app".into())
    }

    #[cfg(target_os = "linux")]
    {
        let terminals: &[(&str, &[&str])] = &[
            ("x-terminal-emulator", &["-e", "claude", "login"]),
            ("gnome-terminal", &["--", "claude", "login"]),
            ("konsole", &["-e", "claude", "login"]),
            ("xfce4-terminal", &["-e", "claude login"]),
            ("xterm", &["-e", "claude", "login"]),
        ];
        for (term, args) in terminals {
            match Command::new(term).args(*args).spawn() {
                Ok(_) => return Ok(term.to_string()),
                Err(_) => continue,
            }
        }
        Err("no terminal emulator found (tried x-terminal-emulator, gnome-terminal, konsole, xfce4-terminal, xterm). Run `claude login` manually.".into())
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
