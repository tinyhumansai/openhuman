//! Loads custom [`AgentDefinition`] files from disk.
//!
//! Custom definitions live as TOML files under `<workspace>/agents/*.toml`,
//! with a fallback to `~/.openhuman/agents/*.toml` for user-global
//! specialists. Each file defines exactly one definition.
//!
//! TOML (rather than YAML) is used for consistency with the rest of
//! OpenHuman's config system, which already depends on the `toml` crate
//! and uses TOML for its main config file.
//!
//! The loader is intentionally lenient: it logs and skips files that fail
//! to parse rather than aborting startup, so a single broken specialist
//! never breaks the rest of the system.

use super::definition::{AgentDefinition, DefinitionSource, PromptSource};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Load all custom definitions from `<workspace>/agents/` and the
/// `~/.openhuman/agents/` fallback. Returns an empty Vec when neither
/// directory exists.
pub fn load_from_workspace(workspace: &Path) -> Result<Vec<AgentDefinition>> {
    let mut out = Vec::new();
    let mut seen_dirs: Vec<PathBuf> = Vec::new();

    let workspace_dir = workspace.join("agents");
    if workspace_dir.is_dir() {
        load_dir(&workspace_dir, &mut out)?;
        seen_dirs.push(workspace_dir);
    }

    if let Some(home_dir) = user_home_agents_dir() {
        if home_dir.is_dir() && !seen_dirs.contains(&home_dir) {
            load_dir(&home_dir, &mut out)?;
        }
    }

    Ok(out)
}

/// Load every `.toml` file in a single directory (non-recursive). Files
/// that fail to parse are logged and skipped.
pub fn load_dir(dir: &Path, out: &mut Vec<AgentDefinition>) -> Result<()> {
    let entries =
        fs::read_dir(dir).with_context(|| format!("reading agents dir {}", dir.display()))?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(
                    dir = %dir.display(),
                    error = %err,
                    "[agent_defs] failed to read directory entry, skipping"
                );
                continue;
            }
        };

        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if ext != "toml" {
            continue;
        }

        match load_file(&path) {
            Ok(def) => {
                tracing::debug!(
                    id = %def.id,
                    path = %path.display(),
                    "[agent_defs] loaded custom definition"
                );
                out.push(def);
            }
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "[agent_defs] failed to load custom definition, skipping"
                );
            }
        }
    }
    Ok(())
}

/// Load a single TOML file as an [`AgentDefinition`]. Stamps `source` to
/// the absolute path.
///
/// Rejects definitions that omit (or leave blank) their `system_prompt`
/// — built-in agents are loaded separately and have their prompts
/// injected by [`crate::openhuman::agent::registry::agents::load_builtins`], so a
/// file-loaded definition that arrives with the
/// [`defaults::empty_inline_prompt`] placeholder is always a caller
/// mistake. Custom definitions must set either
/// `[system_prompt] inline = "…"` or `[system_prompt] file = "…"`.
pub fn load_file(path: &Path) -> Result<AgentDefinition> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut def: AgentDefinition = toml::from_str(&content)
        .with_context(|| format!("parsing {} as AgentDefinition TOML", path.display()))?;
    if let PromptSource::Inline(body) = &def.system_prompt {
        if body.is_empty() {
            bail!(
                "{}: missing `system_prompt` — custom definitions must set an inline string \
                 or a file path",
                path.display()
            );
        }
    }
    def.source = DefinitionSource::File(path.to_path_buf());
    Ok(def)
}

fn user_home_agents_dir() -> Option<PathBuf> {
    // Honour OPENHUMAN_HOME first if set; otherwise ~/.openhuman.
    if let Ok(custom) = std::env::var("OPENHUMAN_HOME") {
        return Some(PathBuf::from(custom).join("agents"));
    }
    match crate::openhuman::config::default_root_openhuman_dir() {
        Ok(dir) => Some(dir.join("agents")),
        Err(error) => {
            tracing::debug!(
                error = %error,
                "[agent-definition-loader] resolving root openhuman dir failed"
            );
            None
        }
    }
}

#[cfg(test)]
#[path = "definition_loader_tests.rs"]
mod tests;
