//! Workspace layout and bootstrap files (CLI `init` and similar entrypoints).

mod schemas;
pub use schemas::{
    all_controller_schemas as all_workspace_controller_schemas,
    all_registered_controllers as all_workspace_registered_controllers,
};

use serde_json::json;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::heartbeat::engine::HeartbeatEngine;
use crate::openhuman::skills::init_skills_dir;
use std::path::Path;

const BOOTSTRAP_FILES: [(&str, &str); 7] = [
    ("AGENTS.md", include_str!("../agent/prompts/AGENTS.md")),
    ("SOUL.md", include_str!("../agent/prompts/SOUL.md")),
    ("TOOLS.md", include_str!("../agent/prompts/TOOLS.md")),
    ("IDENTITY.md", include_str!("../agent/prompts/IDENTITY.md")),
    ("USER.md", include_str!("../agent/prompts/USER.md")),
    (
        "BOOTSTRAP.md",
        include_str!("../agent/prompts/BOOTSTRAP.md"),
    ),
    ("MEMORY.md", include_str!("../agent/prompts/MEMORY.md")),
];

fn ensure_workspace_file(
    workspace_dir: &Path,
    filename: &str,
    contents: &str,
    force: bool,
) -> Result<&'static str, String> {
    let path = workspace_dir.join(filename);
    if path.exists() && !force {
        return Ok("existing");
    }
    std::fs::write(&path, contents)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(if force { "overwritten" } else { "created" })
}

/// Create default dirs, copy bundled prompts, skills README, and heartbeat file.
pub async fn init_workspace(force: bool) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let workspace_dir = config.workspace_dir.clone();

    let mut created_dirs = Vec::new();
    let mut existing_dirs = Vec::new();
    for rel in ["memory", "sessions", "state", "cron"] {
        let dir = workspace_dir.join(rel);
        if dir.exists() {
            existing_dirs.push(dir.display().to_string());
        } else {
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("failed to create directory {}: {e}", dir.display()))?;
            created_dirs.push(dir.display().to_string());
        }
    }

    let mut created_files = Vec::new();
    let mut overwritten_files = Vec::new();
    let mut existing_files = Vec::new();
    for (filename, contents) in BOOTSTRAP_FILES {
        match ensure_workspace_file(&workspace_dir, filename, contents, force)? {
            "created" => created_files.push(workspace_dir.join(filename).display().to_string()),
            "overwritten" => {
                overwritten_files.push(workspace_dir.join(filename).display().to_string())
            }
            _ => existing_files.push(workspace_dir.join(filename).display().to_string()),
        }
    }

    let skills_readme = workspace_dir.join("skills").join("README.md");
    let had_skills_readme = skills_readme.exists();
    let heartbeat = workspace_dir.join("HEARTBEAT.md");
    let had_heartbeat = heartbeat.exists();
    init_skills_dir(&workspace_dir).map_err(|e| format!("failed to initialize skills dir: {e}"))?;
    HeartbeatEngine::ensure_heartbeat_file(&workspace_dir)
        .await
        .map_err(|e| format!("failed to initialize HEARTBEAT.md: {e}"))?;

    if had_skills_readme {
        existing_files.push(skills_readme.display().to_string());
    } else {
        created_files.push(skills_readme.display().to_string());
    }

    if had_heartbeat {
        existing_files.push(heartbeat.display().to_string());
    } else {
        created_files.push(heartbeat.display().to_string());
    }

    Ok(json!({
        "result": {
            "workspace_dir": workspace_dir.display().to_string(),
            "config_path": config.config_path.display().to_string(),
            "directories": {
                "created": created_dirs,
                "existing": existing_dirs
            },
            "files": {
                "created": created_files,
                "overwritten": overwritten_files,
                "existing": existing_files
            }
        },
        "logs": [
            "workspace initialization completed"
        ]
    }))
}
