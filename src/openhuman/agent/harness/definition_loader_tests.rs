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
}

#[test]
fn rejects_definition_with_missing_system_prompt() {
    let ws = fresh_workspace();
    let agents_dir = ws.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    // No `[system_prompt]` table — serde falls back to the empty
    // inline placeholder, which the loader must reject.
    write_toml(
        &agents_dir.join("broken.toml"),
        r#"
id = "broken"
when_to_use = "should be rejected"
"#,
    );
    let defs = load_from_workspace(ws.path()).unwrap();
    assert!(
        defs.is_empty(),
        "expected loader to reject definition without system_prompt"
    );
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

    // Load a baseline registry (no custom overrides) to get the
    // built-in count dynamically — avoids coupling to a hardcoded number.
    let baseline = super::super::definition::AgentDefinitionRegistry::load(
        &tempfile::TempDir::new().unwrap().path().join("empty"),
    )
    .unwrap();
    let expected_count = baseline.len();

    let reg = super::super::definition::AgentDefinitionRegistry::load(ws.path()).unwrap();
    // Same id replaced the built-in `code_executor` in place, so the
    // registry size doesn't grow when the custom TOML collides.
    assert_eq!(reg.len(), expected_count);
    let def = reg.get("code_executor").unwrap();
    assert_eq!(def.when_to_use, "CUSTOM OVERRIDE");
    assert!(matches!(def.source, DefinitionSource::File(_)));
}
