//! Resolution of the sibling `openhuman-core` binary.
//!
//! The desktop shell ships the core as a separate executable next to itself
//! (release) or at `target/debug/openhuman-core` (development). Two features
//! need that path:
//!
//! - the MCP settings panel, which hands the path to MCP clients so they can
//!   spawn the stdio MCP server, and
//! - the service fallback commands, which run `openhuman-core service …` as a
//!   one-shot CLI when the in-process core RPC path is unavailable.
//!
//! Keeping the resolution in one place means both features agree on which
//! binary they are talking to.

use std::path::{Path, PathBuf};

/// Walk up from `start` until we find a directory containing
/// `target/debug/openhuman-core[.exe]`. Returns the full path to the binary
/// when found, or `None` if the tree is exhausted.
pub fn find_debug_binary_walking_up(start: &Path) -> Option<PathBuf> {
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
pub fn resolve_core_binary() -> Result<PathBuf, String> {
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
                        "[core_binary] using OPENHUMAN_CORE_BINARY_PATH={}",
                        env_path
                    );
                    return Ok(p);
                }
                log::warn!(
                    "[core_binary] OPENHUMAN_CORE_BINARY_PATH set to {env_path} but file not found; \
                     falling back to walk"
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
            "[core_binary] dev binary found at {}",
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
        "[core_binary] release binary at {}",
        candidate.display()
    );
    Ok(candidate)
}
