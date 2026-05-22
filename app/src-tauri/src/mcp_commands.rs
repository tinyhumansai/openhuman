//! Tauri commands for MCP server configuration.
//!
//! Exposes two commands to the frontend:
//! - `mcp_resolve_binary_path` — locate the `openhuman-core` binary on disk.
//! - `mcp_open_client_config` — open a supported MCP client's config file in
//!   the system default editor so the user can paste the generated snippet.

use std::path::PathBuf;

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

/// Walk up from `start` until we find a directory containing
/// `target/debug/openhuman-core[.exe]`. Returns the full path to the binary
/// when found, or `None` if the tree is exhausted.
fn find_debug_binary_walking_up(start: &std::path::Path) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let bin_name = "openhuman-core.exe";
    #[cfg(not(target_os = "windows"))]
    let bin_name = "openhuman-core";

    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join("target").join("debug").join(bin_name);
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Resolve the absolute path to the `openhuman-core` binary.
///
/// In dev builds (`cfg!(debug_assertions)`) we:
/// 1. Check `OPENHUMAN_CORE_BINARY_PATH` env var first.
/// 2. Walk up from `current_exe()` looking for `target/debug/openhuman-core`.
///
/// In release builds the binary is a sibling of the shell executable:
/// - macOS: `../MacOS/openhuman-core` relative to the host exe.
/// - Windows / Linux: same directory as the host exe.
fn resolve_binary_path() -> Result<PathBuf, String> {
    log::debug!("[mcp_commands] mcp_resolve_binary_path: resolving binary path");

    #[cfg(target_os = "windows")]
    let bin_name = "openhuman-core.exe";
    #[cfg(not(target_os = "windows"))]
    let bin_name = "openhuman-core";

    if cfg!(debug_assertions) {
        // Dev mode: env override takes priority.
        if let Ok(env_path) = std::env::var("OPENHUMAN_CORE_BINARY_PATH") {
            if !env_path.is_empty() {
                let p = PathBuf::from(&env_path);
                if p.exists() {
                    log::debug!(
                        "[mcp_commands] mcp_resolve_binary_path: using OPENHUMAN_CORE_BINARY_PATH={}",
                        env_path
                    );
                    return Ok(p);
                }
                log::warn!(
                    "[mcp_commands] OPENHUMAN_CORE_BINARY_PATH set to {env_path} but file not found; falling back to walk"
                );
            }
        }

        let exe = std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?;
        let start = exe
            .parent()
            .ok_or_else(|| "current_exe has no parent directory".to_string())?;

        let candidate = find_debug_binary_walking_up(start).ok_or_else(|| {
            format!(
                "could not find target/debug/{bin_name} walking up from {}",
                start.display()
            )
        })?;

        log::debug!(
            "[mcp_commands] mcp_resolve_binary_path: dev binary found at {}",
            candidate.display()
        );
        return Ok(candidate);
    }

    // Release mode: sibling binary.
    let exe = std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?;

    #[cfg(target_os = "macos")]
    let candidate = {
        // macOS .app: Contents/MacOS/<host> → Contents/MacOS/openhuman-core
        exe.parent()
            .map(|p| p.join(bin_name))
            .ok_or_else(|| "current_exe has no parent directory".to_string())?
    };

    #[cfg(not(target_os = "macos"))]
    let candidate = exe
        .parent()
        .map(|p| p.join(bin_name))
        .ok_or_else(|| "current_exe has no parent directory".to_string())?;

    if !candidate.exists() {
        return Err(format!(
            "openhuman-core binary not found at expected path: {}",
            candidate.display()
        ));
    }

    log::debug!(
        "[mcp_commands] mcp_resolve_binary_path: release binary at {}",
        candidate.display()
    );
    Ok(candidate)
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
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // config_path_for_client — pure path resolution tests
    // -------------------------------------------------------------------------

    #[test]
    fn config_path_claude_desktop_macos() {
        let path =
            config_path_for_client("claude-desktop", "macos").expect("should resolve on macos");
        let s = path.display().to_string();
        assert!(
            s.contains("Library/Application Support/Claude/claude_desktop_config.json"),
            "unexpected path: {s}"
        );
    }

    #[test]
    fn config_path_claude_desktop_linux() {
        let path =
            config_path_for_client("claude-desktop", "linux").expect("should resolve on linux");
        let s = path.display().to_string();
        assert!(
            s.contains(".config/Claude/claude_desktop_config.json"),
            "unexpected path: {s}"
        );
    }

    #[test]
    fn config_path_cursor_macos() {
        let path = config_path_for_client("cursor", "macos").expect("should resolve");
        let s = path.display().to_string();
        assert!(s.ends_with(".cursor/mcp.json"), "unexpected path: {s}");
    }

    #[test]
    fn config_path_cursor_linux() {
        let path = config_path_for_client("cursor", "linux").expect("should resolve");
        let s = path.display().to_string();
        assert!(s.ends_with(".cursor/mcp.json"), "unexpected path: {s}");
    }

    #[test]
    fn config_path_codex_all_platforms() {
        for os in &["macos", "windows", "linux"] {
            let path = config_path_for_client("codex", os)
                .unwrap_or_else(|_| panic!("should resolve for os={os}"));
            let s = path.display().to_string();
            assert!(
                s.ends_with(".codex/config.json"),
                "unexpected path for os={os}: {s}"
            );
        }
    }

    #[test]
    fn config_path_zed_macos() {
        let path = config_path_for_client("zed", "macos").expect("should resolve");
        let s = path.display().to_string();
        assert!(
            s.contains("Library/Application Support/Zed/settings.json"),
            "unexpected path: {s}"
        );
    }

    #[test]
    fn config_path_zed_linux() {
        let path = config_path_for_client("zed", "linux").expect("should resolve");
        let s = path.display().to_string();
        assert!(
            s.contains(".config/zed/settings.json"),
            "unexpected path: {s}"
        );
    }

    #[test]
    fn config_path_unknown_client_returns_err() {
        let result = config_path_for_client("unknown-client", "macos");
        assert!(result.is_err(), "unknown client should return Err");
        let err = result.unwrap_err();
        assert!(
            err.contains("Unknown MCP client: unknown-client"),
            "unexpected error message: {err}"
        );
    }

    // -------------------------------------------------------------------------
    // mcp_resolve_binary_path — integration-style test (path must exist in dev)
    // -------------------------------------------------------------------------

    /// In debug builds (the only mode in which `cargo test` runs), the binary
    /// path resolver should either find `OPENHUMAN_CORE_BINARY_PATH` or locate
    /// `target/debug/openhuman-core` by walking up from the test executable.
    ///
    /// We only assert the path *contains* `openhuman-core` — the binary may or
    /// may not exist on disk in a fresh checkout, so we don't assert `Ok` here;
    /// instead we verify the error message is sensible when the file is absent.
    #[test]
    fn binary_path_result_contains_openhuman_core() {
        match resolve_binary_path() {
            Ok(p) => {
                let s = p.display().to_string();
                assert!(
                    s.contains("openhuman-core"),
                    "resolved path should contain 'openhuman-core', got: {s}"
                );
            }
            Err(e) => {
                // Acceptable in a clean CI checkout where the binary hasn't
                // been built yet. The error must be descriptive.
                assert!(
                    e.contains("openhuman-core")
                        || e.contains("current_exe")
                        || e.contains("target"),
                    "error message should reference the binary or path: {e}"
                );
            }
        }
    }

    #[test]
    fn find_debug_binary_returns_none_for_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Walk up from a fresh tempdir in the system temp folder — no ancestor
        // of /tmp (or equivalent) will contain target/debug/openhuman-core.
        let result = find_debug_binary_walking_up(dir.path());
        assert!(
            result.is_none(),
            "expected None walking up from an empty tempdir, got: {result:?}"
        );
    }
}
