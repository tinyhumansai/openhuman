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

use super::definition::{AgentDefinition, DefinitionSource};
use anyhow::{Context, Result};
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
pub fn load_file(path: &Path) -> Result<AgentDefinition> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut def: AgentDefinition = toml::from_str(&content)
        .with_context(|| format!("parsing {} as AgentDefinition TOML", path.display()))?;
    def.source = DefinitionSource::File(path.to_path_buf());
    Ok(def)
}

fn user_home_agents_dir() -> Option<PathBuf> {
    // Honour OPENHUMAN_HOME first if set; otherwise ~/.openhuman.
    if let Ok(custom) = std::env::var("OPENHUMAN_HOME") {
        return Some(PathBuf::from(custom).join("agents"));
    }
    dirs::home_dir().map(|h| h.join(".openhuman").join("agents"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_toml(path: &Path, contents: &str) {
        let mut f = fs::File::create(path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
    }

    fn fresh_workspace() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    // NOTE: TOML parsing is positional. Top-level scalars MUST come
    // before any `[table]` header — once a header opens, every line
    // below it lives inside that table.
    const NOTION_TOML: &str = r#"
id = "notion_specialist"
when_to_use = "Delegate Notion queries to a focused specialist."
display_name = "Notion Specialist"
temperature = 0.4
skill_filter = "notion"
max_iterations = 5

[system_prompt]
inline = "You are the Notion specialist. Use only Notion tools."

[model]
hint = "agentic"
"#;

    #[test]
    fn loads_single_definition_from_workspace() {
        let ws = fresh_workspace();
        let agents_dir = ws.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        write_toml(&agents_dir.join("notion.toml"), NOTION_TOML);

        let defs = load_from_workspace(ws.path()).unwrap();
        assert_eq!(defs.len(), 1);
        let def = &defs[0];
        assert_eq!(def.id, "notion_specialist");
        assert_eq!(def.skill_filter.as_deref(), Some("notion"));
        assert_eq!(def.max_iterations, 5);
        assert!(matches!(def.source, DefinitionSource::File(_)));
    }

    #[test]
    fn empty_when_no_agents_dir() {
        let ws = fresh_workspace();
        let defs = load_from_workspace(ws.path()).unwrap();
        assert!(defs.is_empty());
    }

    #[test]
    fn ignores_non_toml_files() {
        let ws = fresh_workspace();
        let agents_dir = ws.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        write_toml(&agents_dir.join("readme.md"), "not a definition");
        write_toml(&agents_dir.join("notion.toml"), NOTION_TOML);

        let defs = load_from_workspace(ws.path()).unwrap();
        assert_eq!(defs.len(), 1);
    }

    #[test]
    fn skips_malformed_files_without_aborting() {
        let ws = fresh_workspace();
        let agents_dir = ws.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        write_toml(&agents_dir.join("broken.toml"), "id = \"broken\"  [oops");
        write_toml(&agents_dir.join("notion.toml"), NOTION_TOML);

        let defs = load_from_workspace(ws.path()).unwrap();
        // The broken file is skipped; the valid one still loads.
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].id, "notion_specialist");
    }

    #[test]
    fn registry_load_merges_builtins_and_custom() {
        let ws = fresh_workspace();
        let agents_dir = ws.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        write_toml(&agents_dir.join("notion.toml"), NOTION_TOML);

        let reg = super::super::definition::AgentDefinitionRegistry::load(ws.path()).unwrap();
        // The built-in set is allowed to grow over time (new archetypes,
        // additional synthetic definitions), so assert presence of the
        // specific ids we care about rather than a fixed total count.
        assert!(
            reg.len() > 1,
            "expected at least one built-in plus the custom definition"
        );
        assert!(reg.get("notion_specialist").is_some());
        assert!(reg.get("code_executor").is_some());
        assert!(reg.get("fork").is_some());
    }

    #[test]
    fn custom_definition_overrides_same_id_builtin() {
        let ws = fresh_workspace();
        let agents_dir = ws.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        // Override the built-in `code_executor` with a custom one.
        write_toml(
            &agents_dir.join("code_executor.toml"),
            r#"
id = "code_executor"
when_to_use = "CUSTOM OVERRIDE"

[system_prompt]
inline = "custom prompt"

[tools]
wildcard = {}
"#,
        );

        let reg = super::super::definition::AgentDefinitionRegistry::load(ws.path()).unwrap();
        // Still 9 — same id replaced built-in in place.
        assert_eq!(reg.len(), 9);
        let def = reg.get("code_executor").unwrap();
        assert_eq!(def.when_to_use, "CUSTOM OVERRIDE");
        assert!(matches!(def.source, DefinitionSource::File(_)));
    }
}
