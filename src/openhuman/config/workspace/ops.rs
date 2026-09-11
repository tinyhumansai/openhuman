use serde_json::json;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::skills::init_workflows_dir;
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
/// Persona Pack RPCs (`src/openhuman/config/workspace/rpc.rs`) treat membership here
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

/// Records `path` in the created / existing list according to what actually
/// happened, given whether it existed before the initialising call.
///
/// A path that still does not exist afterwards is recorded in **neither** list.
/// That case is reachable whenever the code responsible for creating it is
/// compiled out — the `skills` gate's no-op `init_workflows_dir` being the
/// live example — and reporting it as "created" would make `workspace_init`
/// claim work it never did.
fn classify_workspace_entry(
    path: &Path,
    existed_before: bool,
    created: &mut Vec<String>,
    existing: &mut Vec<String>,
) {
    if !path.exists() {
        return;
    }
    if existed_before {
        existing.push(path.display().to_string());
    } else {
        created.push(path.display().to_string());
    }
}

/// Create default dirs, copy bundled prompts, and the skills README.
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
    init_workflows_dir(&workspace_dir)
        .map_err(|e| format!("failed to initialize skills dir: {e}"))?;

    // Report what the call actually did, not what it was expected to do.
    //
    // These two were previously classified from the pre-check alone, which
    // reports a file as "created" whenever it did not exist beforehand — even
    // if nothing subsequently created it. That is wrong in any build where
    // `skills` is compiled out: `init_workflows_dir` resolves to the stub,
    // which is a deliberate no-op, so `skills/README.md` is never written and
    // every call reports it as freshly created, forever. Re-checking existence
    // afterwards makes all three lists honest: absent stays absent.
    classify_workspace_entry(
        &skills_readme,
        had_skills_readme,
        &mut created_files,
        &mut existing_files,
    );

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
