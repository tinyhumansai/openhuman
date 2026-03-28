//! Dev-time path resolution when the repo root is not the process cwd.

use std::path::{Path, PathBuf};

/// Locate `rust-core/ai` by walking up from `cwd` (handles repo root, `app/`, `app/src-tauri/`, etc.).
pub fn rust_core_ai_dir(cwd: &Path) -> Option<PathBuf> {
    for up in 0..=3 {
        let mut base = cwd.to_path_buf();
        let mut ok = true;
        for _ in 0..up {
            if !base.pop() {
                ok = false;
                break;
            }
        }
        if !ok {
            continue;
        }
        let candidate = base.join("rust-core").join("ai");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}
