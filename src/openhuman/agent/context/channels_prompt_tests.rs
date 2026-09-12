use super::*;
use crate::openhuman::skills::{Workflow, WorkflowFrontmatter, WorkflowScope};
use tempfile::tempdir;

fn skill(name: &str, location: Option<std::path::PathBuf>) -> Workflow {
    Workflow {
        name: name.to_string(),
        dir_name: name.to_string(),
        description: format!("{name} description"),
        version: "1.0.0".into(),
        author: None,
        tags: vec![],
        platforms: vec![],
        related_skills: vec![],
        source_format: "openhuman".to_string(),
        tools: vec![],
        prompts: vec![],
        location,
        frontmatter: WorkflowFrontmatter::default(),
        resources: vec![],
        scope: WorkflowScope::Project,
        legacy: false,
        warnings: vec![],
    }
}

#[test]
fn inject_workspace_file_truncates_at_utf8_boundary_and_emits_read_hint() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("SOUL.md"), "é".repeat(10)).unwrap();

    let mut prompt = String::new();
    inject_workspace_file(&mut prompt, tmp.path(), "SOUL.md", 5);

    assert!(prompt.contains("### SOUL.md"));
    assert!(prompt.contains("truncated at 5 chars"));
    assert!(prompt.is_char_boundary(prompt.len()));
    assert!(prompt.contains("read` for full file") || prompt.contains("read` for full file]"));
}

#[test]
fn inject_workspace_file_emits_missing_marker_for_bootstrap_files() {
    let tmp = tempdir().unwrap();
    let mut prompt = String::new();

    inject_workspace_file(&mut prompt, tmp.path(), "IDENTITY.md", 100);

    assert!(prompt.contains("[File not found: IDENTITY.md]"));
}

#[test]
fn load_openclaw_bootstrap_files_omits_missing_optional_files() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("SOUL.md"), "Soul").unwrap();
    std::fs::write(tmp.path().join("IDENTITY.md"), "Identity").unwrap();

    let mut prompt = String::new();
    load_openclaw_bootstrap_files(
        &mut prompt,
        tmp.path(),
        100,
        PromptIdentityOverride::default(),
    );

    assert!(prompt.contains("### SOUL.md"));
    assert!(prompt.contains("### IDENTITY.md"));
    assert!(!prompt.contains("PROFILE.md"));
    assert!(!prompt.contains("MEMORY.md"));
}

#[test]
fn build_system_prompt_uses_generic_channel_copy_when_channel_name_absent() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("SOUL.md"), "Soul").unwrap();
    std::fs::write(tmp.path().join("IDENTITY.md"), "Identity").unwrap();

    let prompt = build_system_prompt(tmp.path(), "test-model", &[], &[], None, None);

    assert!(prompt.contains("You are running as a messaging bot."));
    assert!(prompt.contains(
        "When someone messages you, your response is automatically sent back to the same platform."
    ));
    assert!(!prompt.contains("running as a Discord bot"));
}

#[test]
fn build_system_prompt_uses_workspace_skill_location_fallback() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("SOUL.md"), "Soul").unwrap();
    std::fs::write(tmp.path().join("IDENTITY.md"), "Identity").unwrap();

    let prompt = build_system_prompt(
        tmp.path(),
        "test-model",
        &[("search", "Find facts")],
        &[skill("research-helper", None)],
        None,
        Some("Telegram"),
    );

    assert!(prompt.contains("<name>research-helper</name>"));
    assert!(prompt.contains(
        &tmp.path()
            .join("skills")
            .join("research-helper")
            .join("SKILL.md")
            .display()
            .to_string()
    ));
    assert!(prompt.contains("running as a Telegram bot"));
}

// ── Identity override (#6027) ────────────────────────────────────────

fn identity_workspace() -> tempfile::TempDir {
    let tmp = tempdir().unwrap();
    std::fs::write(
        tmp.path().join("SOUL.md"),
        "I am the conflicting workspace-root identity.",
    )
    .unwrap();
    std::fs::write(tmp.path().join("IDENTITY.md"), "Name: OpenHuman").unwrap();
    std::fs::write(tmp.path().join("MEMORY.md"), "shared root memory marker").unwrap();
    tmp
}

#[test]
fn identity_override_replaces_root_soul_and_memory() {
    let tmp = identity_workspace();
    let prompt = build_system_prompt_with_identity(
        tmp.path(),
        "model",
        &[],
        &[],
        None,
        Some("Discord"),
        PromptIdentityOverride {
            soul_md: Some("I am Alice, a meticulous archivist."),
            memory_md: Some("alice private memory marker"),
        },
        ProjectContextPlacement::Inline,
    );

    assert!(prompt.contains("### SOUL.md"), "SOUL slot keeps its header");
    assert!(prompt.contains("I am Alice, a meticulous archivist."));
    assert!(
        !prompt.contains("conflicting workspace-root identity"),
        "profile SOUL.md must replace, not accompany, the root file"
    );
    assert!(
        prompt.contains("Name: OpenHuman"),
        "IDENTITY.md is not overridable and stays the root file"
    );
    assert!(prompt.contains("### MEMORY.md"));
    assert!(prompt.contains("alice private memory marker"));
    assert!(
        !prompt.contains("shared root memory marker"),
        "profile MEMORY.md must replace the root one"
    );
}

#[test]
fn memory_override_renders_even_when_root_memory_is_absent() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("SOUL.md"), "root soul").unwrap();
    let prompt = build_system_prompt_with_identity(
        tmp.path(),
        "model",
        &[],
        &[],
        None,
        None,
        PromptIdentityOverride {
            soul_md: None,
            memory_md: Some("profile-only memory"),
        },
        ProjectContextPlacement::Inline,
    );
    assert!(prompt.contains("root soul"), "no soul override → root soul");
    assert!(prompt.contains("### MEMORY.md"));
    assert!(prompt.contains("profile-only memory"));
}

#[test]
fn empty_identity_override_matches_root_render_byte_for_byte() {
    let tmp = identity_workspace();
    let tools = [("shell", "Run commands")];
    let root = build_system_prompt(tmp.path(), "model", &tools, &[], Some(50), Some("Discord"));
    let overridden = build_system_prompt_with_identity(
        tmp.path(),
        "model",
        &tools,
        &[],
        Some(50),
        Some("Discord"),
        PromptIdentityOverride::default(),
        ProjectContextPlacement::Inline,
    );
    assert_eq!(
        root, overridden,
        "the wrapper and an empty override are one render"
    );
}

#[test]
fn omitted_placement_leaves_the_project_context_to_the_caller() {
    let tmp = identity_workspace();
    let identity = PromptIdentityOverride {
        soul_md: Some("I am Alice, a meticulous archivist."),
        memory_md: None,
    };
    let mut prompt = build_system_prompt_with_identity(
        tmp.path(),
        "model",
        &[],
        &[],
        None,
        Some("Discord"),
        identity,
        ProjectContextPlacement::Omitted,
    );
    assert!(
        !prompt.contains("## Project Context"),
        "omitted → nothing rendered"
    );
    assert!(!prompt.contains("I am Alice"));
    assert!(
        prompt.contains("## Channel Capabilities"),
        "the rest still renders"
    );

    prompt.push_str("tool schemas go here\n");
    render_project_context(&mut prompt, tmp.path(), None, identity);
    let ctx = prompt
        .find("## Project Context")
        .expect("appended by the caller");
    assert!(ctx > prompt.find("tool schemas go here").unwrap());
    assert!(prompt.contains("I am Alice, a meticulous archivist."));
    assert!(!prompt.contains("conflicting workspace-root identity"));
    assert!(prompt.contains("Name: OpenHuman"));
}
