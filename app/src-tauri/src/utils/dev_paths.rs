//! Dev-time path resolution for bundled / repo AI prompts (desktop host).

use std::path::{Path, PathBuf};

/// Best-effort path to repo `src/openhuman/agent/prompts` when running from a checkout.
pub fn repo_ai_prompts_dir(cwd: &Path) -> Option<PathBuf> {
    let prompts = cwd
        .join("src")
        .join("openhuman")
        .join("agent")
        .join("prompts");
    if prompts.is_dir() {
        return Some(prompts);
    }
    let from_app = cwd
        .join("..")
        .join("src")
        .join("openhuman")
        .join("agent")
        .join("prompts");
    if from_app.is_dir() {
        return Some(from_app);
    }
    let app_prompts = cwd
        .join("app")
        .join("src")
        .join("openhuman")
        .join("agent")
        .join("prompts");
    if app_prompts.is_dir() {
        return Some(app_prompts);
    }
    None
}

/// Bundled OpenClaw-style prompts inside the packaged app resource dir.
pub fn bundled_openclaw_prompts_dir(resource_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        resource_dir.join("openhuman").join("agent").join("prompts"),
        resource_dir.join("ai").join("prompts"),
        resource_dir.join("_up_").join("ai").join("prompts"),
    ];
    for p in candidates {
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}
