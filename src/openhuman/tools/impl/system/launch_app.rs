//! Tool: launch_app — open a named application on the user's desktop.
//!
//! A dedicated, narrow-scope alternative to using the `shell` tool with
//! `open -a <App>` / `xdg-open` / `Start-Process`. It carries no shell
//! injection risk and accepts **named applications only** (URI schemes like
//! `spotify:` / `mailto:` are rejected — see `validate_app_name`).
//!
//! Being injection-safe does NOT make it side-effect-free: opening an app
//! window (and, on Linux/Windows, potentially firing a registered URI handler)
//! is an externally-observable action on the user's machine. So the tool is an
//! external-effect tool (`external_effect() == true`) and routes through the
//! `ApprovalGate` before executing, like `shell` — it is NOT always-allow.
//!
//! Platform dispatch:
//!   macOS   — `open -a "<app_name>"` (falls back to `open "<app_name>"`)
//!   Linux   — `gtk-launch "<app_name>"`, fallback `xdg-open "<app_name>"`
//!   Windows — `Start-Process "<app_name>"`

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::process::Stdio;

pub struct LaunchAppTool;

impl LaunchAppTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LaunchAppTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Reject names that look like path traversal or contain shell metacharacters.
fn validate_app_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("app_name must not be empty".into());
    }
    if trimmed.len() > 128 {
        return Err("app_name too long (max 128 chars)".into());
    }
    // No path separators or traversal sequences.
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err(format!(
            "app_name '{trimmed}' looks like a path; supply a bare application name instead \
             (e.g. 'Music', 'Spotify')"
        ));
    }
    // Reject shell metacharacters — not needed here since we bypass the shell,
    // but guard against accidental misuse of the API.
    for ch in ['$', '`', '|', '&', ';', '>', '<', '!', '(', ')', '\n', '\r'] {
        if trimmed.contains(ch) {
            return Err(format!("app_name contains disallowed character '{ch}'"));
        }
    }
    // Reject URI schemes (e.g. `spotify:`, `mailto:`, `slack:`, `https:`). On
    // Linux/Windows the launcher fallbacks (`xdg-open`/`Start-Process`) would
    // fire an arbitrary registered URI handler — exactly the ungated
    // network/system reach that `open`/`xdg-open`/`start` were kept out of
    // READ_ONLY_BASES to avoid. This tool is "named applications only".
    if is_uri_scheme(trimmed) {
        return Err(format!(
            "app_name '{trimmed}' looks like a URI scheme; this tool launches named \
             applications only, not URIs/handlers"
        ));
    }
    Ok(())
}

/// True if `s` begins with a URI scheme per RFC 3986: `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"`.
fn is_uri_scheme(s: &str) -> bool {
    let Some(colon) = s.find(':') else {
        return false;
    };
    if colon == 0 {
        return false;
    }
    let scheme = &s[..colon];
    let mut chars = scheme.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    scheme
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

#[async_trait]
impl Tool for LaunchAppTool {
    fn name(&self) -> &str {
        "launch_app"
    }

    fn description(&self) -> &str {
        "Open a named application on the user's desktop. Supply the app's display name \
         (e.g. 'Music', 'Spotify', 'Safari', 'Calculator', 'VS Code'). \
         Works on macOS, Linux, and Windows. \
         Use this instead of the shell tool whenever the goal is simply to open an app."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "app_name": {
                    "type": "string",
                    "description": "Display name of the application to open \
                                    (e.g. 'Music', 'Spotify', 'Google Chrome'). \
                                    Do not supply a file path — use the bare name."
                }
            },
            "required": ["app_name"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Launching an app executes a process / opens a window on the user's
        // machine — an execution-class action, not a read.
        PermissionLevel::Execute
    }

    fn external_effect(&self) -> bool {
        // Opening an application is an externally-observable side effect, so the
        // harness routes this through the ApprovalGate before execute() — same
        // contract as `shell`. Not always-allow.
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        let app_name = args
            .get("app_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        log::info!("[launch_app] ▶ execute called app_name={app_name:?} raw_args={args}");

        if let Err(reason) = validate_app_name(&app_name) {
            log::warn!("[launch_app] ✗ validation failed app_name={app_name:?} reason={reason}");
            return Ok(ToolResult::error(reason));
        }

        log::info!("[launch_app] ✓ validation passed — dispatching to platform launcher");

        let result = launch_platform(&app_name).await;

        match result {
            Ok(msg) => {
                log::info!("[launch_app] ✓ launch succeeded app_name={app_name:?} msg={msg:?}");
                Ok(ToolResult::success(msg))
            }
            Err(err) => {
                log::warn!("[launch_app] ✗ launch failed app_name={app_name:?} error={err}");
                Ok(ToolResult::error(format!(
                    "Could not open '{app_name}': {err}"
                )))
            }
        }
    }
}

/// Platform-specific launch dispatch. Returns a human-readable success message.
///
/// `pub(crate)` so the `automate` inner loop (`accessibility::automate`) can
/// launch an app as one of its steps without duplicating the platform branches
/// or routing back through the full tool surface.
pub(crate) async fn launch_platform(app_name: &str) -> Result<String, String> {
    log::info!(
        "[launch_app] platform={} dispatching launch for app_name={app_name:?}",
        std::env::consts::OS
    );

    #[cfg(target_os = "macos")]
    return launch_macos(app_name).await;

    #[cfg(target_os = "linux")]
    return launch_linux(app_name).await;

    #[cfg(target_os = "windows")]
    return launch_windows(app_name).await;

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    Err("launch_app is not supported on this platform".into())
}

#[cfg(target_os = "macos")]
async fn launch_macos(app_name: &str) -> Result<String, String> {
    log::info!("[launch_app] macOS: running `open -a {app_name:?}`");

    // `open -a "App Name"` resolves by display name via LaunchServices.
    let output = tokio::process::Command::new("open")
        .arg("-a")
        .arg(app_name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("failed to invoke `open`: {e}"))?;

    log::info!(
        "[launch_app] macOS: `open -a` exit={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );

    if output.status.success() {
        return Ok(format!("Opened '{app_name}'."));
    }

    log::info!("[launch_app] macOS: primary failed — trying fallback `open {app_name:?}`");

    // Fallback: `open "<App Name>"` — works for bundle names and some URIs.
    let fallback = tokio::process::Command::new("open")
        .arg(app_name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|e| format!("failed to invoke `open` (fallback): {e}"))?;

    log::info!("[launch_app] macOS: fallback exit={fallback}");

    if fallback.success() {
        Ok(format!("Opened '{app_name}'."))
    } else {
        Err(format!(
            "`open -a \"{app_name}\"` failed (exit {}) — check the app name matches its title in /Applications",
            output.status
        ))
    }
}

#[cfg(target_os = "linux")]
async fn launch_linux(app_name: &str) -> Result<String, String> {
    // `gtk-launch` takes a .desktop file ID (e.g. "google-chrome"), NOT a
    // human-readable display name ("Google Chrome"). Try the name as given
    // first, then a best-effort desktop-id derived from the display name
    // (lowercased, spaces → hyphens). `xdg-open` does NOT launch apps by
    // name — it only opens URIs/paths in the default handler — so it's a
    // last resort for app_name values that happen to be a URI.
    let desktop_id = app_name.to_lowercase().replace(' ', "-");
    let mut candidates = vec![app_name.to_string()];
    if desktop_id != app_name {
        candidates.push(desktop_id);
    }

    for candidate in &candidates {
        let gtk = tokio::process::Command::new("gtk-launch")
            .arg(candidate)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        if let Ok(s) = gtk {
            if s.success() {
                return Ok(format!("Opened '{app_name}'."));
            }
        }
    }

    // Fallback for URI-shaped inputs only (xdg-open won't resolve app names).
    let xdg = tokio::process::Command::new("xdg-open")
        .arg(app_name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|e| format!("failed to invoke `xdg-open`: {e}"))?;

    if xdg.success() {
        Ok(format!("Opened '{app_name}'."))
    } else {
        Err(format!(
            "Could not launch '{app_name}' on Linux. gtk-launch needs a .desktop \
             ID (e.g. 'google-chrome'); xdg-open only opens URIs/paths, not app names. \
             Try the .desktop ID, or supply a URI."
        ))
    }
}

#[cfg(target_os = "windows")]
async fn launch_windows(app_name: &str) -> Result<String, String> {
    // The app name is passed through an env var (`OH_LAUNCH_APP`) and never
    // string-interpolated into the script, so a name containing a quote cannot
    // break out of the command. `validate_app_name` already blocks shell
    // metacharacters; the static script + env passing is belt-and-suspenders.
    //
    // Resolution order:
    //   1. `Start-Process -FilePath <name>` — resolves PATH executables,
    //      registered App Paths (e.g. "Spotify" desktop), and URIs ("spotify:").
    //   2. Fallback for Store/UWP apps that have no plain exe: match by display
    //      name via `Get-StartApps` and launch by AUMID
    //      (`shell:AppsFolder\<AppID>`), e.g. the Store "Media Player".
    const PS_LAUNCH: &str = "\
        $ErrorActionPreference='Stop'; \
        $n=$env:OH_LAUNCH_APP; \
        try { Start-Process -FilePath $n } \
        catch { \
          $app = Get-StartApps | Where-Object { $_.Name -like \"*$n*\" } | Select-Object -First 1; \
          if ($app) { Start-Process -FilePath ('shell:AppsFolder\\' + $app.AppID) } \
          else { throw } \
        }";

    log::info!(
        "[launch_app] windows: launching app_name={app_name:?} (Start-Process, Store fallback)"
    );

    let output = tokio::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", PS_LAUNCH])
        .env("OH_LAUNCH_APP", app_name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("failed to invoke PowerShell: {e}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    log::info!(
        "[launch_app] windows: exit={} stderr={}",
        output.status,
        stderr.trim()
    );

    if output.status.success() {
        Ok(format!("Opened '{app_name}'."))
    } else if stderr.trim().is_empty() {
        Err(format!(
            "could not open '{app_name}' (Start-Process and Store-app lookup both failed)"
        ))
    } else {
        Err(format!("could not open '{app_name}': {}", stderr.trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_permission() {
        let tool = LaunchAppTool::new();
        assert_eq!(tool.name(), "launch_app");
        // Execution-class + external effect so it routes through the ApprovalGate.
        assert_eq!(tool.permission_level(), PermissionLevel::Execute);
        assert!(tool.external_effect());
    }

    #[test]
    fn validate_rejects_uri_schemes() {
        // URI schemes would fire arbitrary registered handlers via the
        // Linux/Windows launcher fallbacks — reject them (named apps only).
        assert!(validate_app_name("spotify:track/123").is_err());
        assert!(validate_app_name("mailto:a@b.com").is_err());
        assert!(validate_app_name("slack:").is_err());
        assert!(validate_app_name("https://evil.example").is_err());
        assert!(validate_app_name("x-custom-scheme:payload").is_err());
    }

    #[test]
    fn schema_requires_app_name() {
        let schema = LaunchAppTool::new().parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "app_name"));
    }

    #[test]
    fn validate_rejects_empty() {
        assert!(validate_app_name("").is_err());
        assert!(validate_app_name("   ").is_err());
    }

    #[test]
    fn validate_rejects_paths() {
        assert!(validate_app_name("/Applications/Music.app").is_err());
        assert!(validate_app_name("../etc/passwd").is_err());
    }

    #[test]
    fn validate_rejects_metacharacters() {
        assert!(validate_app_name("Music; rm -rf /").is_err());
        assert!(validate_app_name("$(evil)").is_err());
    }

    #[test]
    fn validate_accepts_normal_names() {
        assert!(validate_app_name("Music").is_ok());
        assert!(validate_app_name("Google Chrome").is_ok());
        assert!(validate_app_name("VS Code").is_ok());
        assert!(validate_app_name("Spotify").is_ok());
    }

    #[tokio::test]
    async fn returns_error_for_empty_app_name() {
        let result = LaunchAppTool::new()
            .execute(json!({"app_name": ""}))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn returns_error_for_path_traversal() {
        let result = LaunchAppTool::new()
            .execute(json!({"app_name": "/Applications/Music.app"}))
            .await
            .unwrap();
        assert!(result.is_error);
    }
}
