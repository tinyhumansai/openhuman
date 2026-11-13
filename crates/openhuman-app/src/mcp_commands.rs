//! Tauri commands for MCP server configuration.
//!
//! Exposes two commands to the frontend:
//! - `mcp_resolve_binary_path` — locate the `openhuman-core` binary on disk.
//! - `mcp_open_client_config` — open a supported MCP client's config file in
//!   the system default editor so the user can paste the generated snippet.

use std::path::PathBuf;

use crate::core_binary::resolve_core_binary;

/// Information returned to the frontend about the MCP server binary.
#[derive(Debug, Clone, serde::Serialize)]
pub struct McpBinaryInfo {
    /// Absolute path to the `openhuman-core` binary.
    pub path: String,
    /// OS string: `"macos"` | `"windows"` | `"linux"`.
    pub os: String,
}

/// Compute the current platform string at compile time.
fn current_os() -> &'static str {
    #[cfg(target_os = "macos")]
    return "macos";
    #[cfg(target_os = "windows")]
    return "windows";
    #[cfg(target_os = "linux")]
    return "linux";
}

/// Resolve the absolute path to the `openhuman-core` binary.
///
/// Thin wrapper over the shared resolver in [`crate::core_binary`] so the MCP
/// panel and the service fallback commands always agree on the binary.
fn resolve_binary_path() -> Result<PathBuf, String> {
    log::debug!("[mcp_commands] mcp_resolve_binary_path: resolving binary path");
    let path = resolve_core_binary()?;
    log::debug!(
        "[mcp_commands] mcp_resolve_binary_path: resolved path={}",
        path.display()
    );
    Ok(path)
}

/// Tauri command — resolve the `openhuman-core` binary path and OS name.
///
/// The frontend uses the returned path to generate client config JSON snippets
/// that tell MCP clients (Claude Desktop, Cursor, Codex, Zed) how to spawn the
/// stdio MCP server.
#[tauri::command]
pub fn mcp_resolve_binary_path() -> Result<McpBinaryInfo, String> {
    log::debug!("[mcp_commands] mcp_resolve_binary_path: command entry");
    let path = resolve_binary_path()?;
    let info = McpBinaryInfo {
        path: path.display().to_string(),
        os: current_os().to_string(),
    };
    log::debug!(
        "[mcp_commands] mcp_resolve_binary_path: resolved path={} os={}",
        info.path,
        info.os
    );
    Ok(info)
}

/// Return the OS-specific config file path for a given MCP client.
///
/// Extracted as a pure function so it can be tested independently of the Tauri
/// command wrapper (which calls `open`/`xdg-open`).
pub fn config_path_for_client(client: &str, os: &str) -> Result<PathBuf, String> {
    let home = directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .ok_or_else(|| "could not determine home directory".to_string())?;

    let path = match (client, os) {
        // Claude Desktop
        ("claude-desktop", "macos") => {
            home.join("Library/Application Support/Claude/claude_desktop_config.json")
        }
        ("claude-desktop", "windows") => {
            // %APPDATA%\Claude\claude_desktop_config.json
            let appdata = std::env::var("APPDATA")
                .unwrap_or_else(|_| home.join("AppData/Roaming").display().to_string());
            PathBuf::from(appdata)
                .join("Claude")
                .join("claude_desktop_config.json")
        }
        ("claude-desktop", _) => {
            // Linux and other Unix
            home.join(".config/Claude/claude_desktop_config.json")
        }

        // Cursor
        ("cursor", "windows") => {
            let userprofile =
                std::env::var("USERPROFILE").unwrap_or_else(|_| home.display().to_string());
            PathBuf::from(userprofile).join(".cursor").join("mcp.json")
        }
        ("cursor", _) => home.join(".cursor/mcp.json"),

        // Codex — same path on all platforms
        ("codex", _) => home.join(".codex/config.json"),

        // Zed
        ("zed", "macos") => home.join("Library/Application Support/Zed/settings.json"),
        ("zed", "windows") => {
            let appdata = std::env::var("APPDATA")
                .unwrap_or_else(|_| home.join("AppData/Roaming").display().to_string());
            PathBuf::from(appdata).join("Zed").join("settings.json")
        }
        ("zed", _) => home.join(".config/zed/settings.json"),

        _ => {
            return Err(format!("Unknown MCP client: {client}"));
        }
    };

    Ok(path)
}

/// Tauri command — open a supported MCP client's config file in the system
/// default editor. Creates the file (and parent dirs) if it does not exist.
///
/// Supported `client` values: `"claude-desktop"`, `"cursor"`, `"codex"`, `"zed"`.
#[tauri::command]
pub fn mcp_open_client_config(client: String) -> Result<(), String> {
    log::debug!("[mcp_commands] mcp_open_client_config: client={client}");

    let os = current_os();
    let path = config_path_for_client(&client, os)?;

    log::debug!(
        "[mcp_commands] mcp_open_client_config: resolved path={} for client={}",
        path.display(),
        client
    );

    // Ensure the file exists so the editor has something to open.
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "failed to create config directory {}: {e}",
                    parent.display()
                )
            })?;
        }
        std::fs::write(&path, b"{}")
            .map_err(|e| format!("failed to create config file {}: {e}", path.display()))?;
        log::debug!(
            "[mcp_commands] mcp_open_client_config: created empty file at {}",
            path.display()
        );
    }

    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(&path).spawn();

    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(&path).spawn();

    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(&path).spawn();

    result
        .map(|_| {
            log::debug!(
                "[mcp_commands] mcp_open_client_config: opened {} for client={}",
                path.display(),
                client
            );
        })
        .map_err(|e| {
            format!(
                "failed to open config file {} for client {client}: {e}",
                path.display()
            )
        })
}

#[cfg(test)]
#[path = "mcp_commands_tests.rs"]
mod tests;
