//! Resolve OpenClaw / AI prompt directories for bundled and dev layouts.

use std::path::{Path, PathBuf};

/// OpenClaw markdown directory inside a bundled resource dir.
pub fn bundled_openclaw_prompts_dir(resource_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        resource_dir.join("prompts"),
        resource_dir.join("ai"),
        resource_dir.join("src").join("ai").join("prompts"),
    ];
    for p in candidates {
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

/// Locate `src/ai/prompts` by walking up from `cwd`.
pub fn repo_ai_prompts_dir(cwd: &Path) -> Option<PathBuf> {
    for up in 0..=8 {
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
        let candidate = base.join("src").join("ai").join("prompts");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}
