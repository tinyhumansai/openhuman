use crate::core::runtime::DispatchCtx;
use serde_json::json;

use std::path::Path;

const BOOTSTRAP_FILES: [(&str, &str); 2] = [
    ("SOUL.md", include_str!("../../agent/prompts/SOUL.md")),
    (
        "IDENTITY.md",
        include_str!("../../agent/prompts/IDENTITY.md"),
    ),
];

/// Bundled default contents for a bootstrap workspace file, or `None` when the
/// name is not one of the files shipped in [`BOOTSTRAP_FILES`].
///
/// This is the single source of truth for both "which workspace files may be
/// edited from the Persona surface" and "what to restore on reset" — the
/// Persona Pack RPCs (`crates/openhuman-core/src/config/workspace/rpc.rs`) treat membership here
/// as the editable allowlist so a caller can never read or clobber an
/// arbitrary path under the workspace.
pub fn bundled_default_contents(filename: &str) -> Option<&'static str> {
    BOOTSTRAP_FILES
        .iter()
        .find(|(name, _)| *name == filename)
        .map(|(_, contents)| *contents)
}

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

/// Create default dirs and copy bundled prompts.
pub async fn init_workspace(ctx: &DispatchCtx, force: bool) -> Result<serde_json::Value, String> {
    let config = ctx.load_config().await?;
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

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
