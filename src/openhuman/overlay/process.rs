//! Discovery and lifecycle management for the `openhuman-overlay` binary.
//!
//! The overlay is a separate Tauri application that provides a transparent
//! floating panel with voice transcription, autocomplete debug info, and
//! Globe/Fn hotkey toggling. The core process launches it as a fire-and-forget
//! child so that it appears automatically when the core RPC server starts.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Attempt to find and spawn the overlay binary.
///
/// This is best-effort: if the binary is not found or fails to launch, a
/// warning is logged and the core continues normally.
pub fn spawn_overlay() {
    let Some(overlay_bin) = find_overlay_binary() else {
        log::debug!("[overlay] openhuman-overlay binary not found — skipping overlay launch");
        return;
    };

    log::info!(
        "[overlay] launching overlay: {}",
        overlay_bin.display()
    );

    match Command::new(&overlay_bin)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            log::info!(
                "[overlay] overlay process spawned (pid={})",
                child.id()
            );
        }
        Err(err) => {
            log::warn!(
                "[overlay] failed to spawn overlay at {}: {err}",
                overlay_bin.display()
            );
        }
    }
}

/// Search for the `openhuman-overlay` binary in standard locations.
///
/// Search order:
/// 1. `OPENHUMAN_OVERLAY_BIN` env var (explicit override)
/// 2. Next to the current executable (`openhuman-overlay` / `openhuman-overlay.exe`)
/// 3. macOS: inside the `.app` bundle Resources directory
/// 4. Dev builds: `overlay/src-tauri/target/debug/openhuman-overlay`
fn find_overlay_binary() -> Option<PathBuf> {
    // 1. Explicit env var override
    if let Ok(path) = std::env::var("OPENHUMAN_OVERLAY_BIN") {
        let candidate = PathBuf::from(&path);
        if candidate.exists() {
            log::debug!("[overlay] found via OPENHUMAN_OVERLAY_BIN: {}", candidate.display());
            return Some(candidate);
        }
        log::debug!(
            "[overlay] OPENHUMAN_OVERLAY_BIN set but path does not exist: {path}"
        );
    }

    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    // 2. Next to the current executable
    let standalone = exe_dir.join(overlay_binary_name());
    if standalone.exists() {
        log::debug!("[overlay] found next to exe: {}", standalone.display());
        return Some(standalone);
    }

    // 3. macOS: Resources directory inside the .app bundle
    #[cfg(target_os = "macos")]
    {
        if let Some(resources_dir) = exe_dir.parent().map(|p| p.join("Resources")) {
            let in_resources = resources_dir.join(overlay_binary_name());
            if in_resources.exists() {
                log::debug!("[overlay] found in Resources: {}", in_resources.display());
                return Some(in_resources);
            }
        }
    }

    // 4. Dev builds: walk up from current exe to find overlay/src-tauri/target
    if cfg!(debug_assertions) {
        // Try relative to CARGO_MANIFEST_DIR at compile time
        let dev_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("overlay")
            .join("src-tauri")
            .join("target")
            .join("debug")
            .join(overlay_binary_name());
        if dev_path.exists() {
            log::debug!("[overlay] found dev build: {}", dev_path.display());
            return Some(dev_path);
        }

        // Also check for the macOS .app bundle in dev
        #[cfg(target_os = "macos")]
        {
            let dev_app_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("overlay")
                .join("src-tauri")
                .join("target")
                .join("debug")
                .join("bundle")
                .join("macos")
                .join("openhuman-overlay.app")
                .join("Contents")
                .join("MacOS")
                .join("openhuman-overlay");
            if dev_app_path.exists() {
                log::debug!("[overlay] found dev .app bundle: {}", dev_app_path.display());
                return Some(dev_app_path);
            }
        }
    }

    None
}

fn overlay_binary_name() -> &'static str {
    #[cfg(windows)]
    {
        "openhuman-overlay.exe"
    }
    #[cfg(not(windows))]
    {
        "openhuman-overlay"
    }
}
