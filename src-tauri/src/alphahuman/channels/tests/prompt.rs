use super::common::make_workspace;
use super::super::prompt::{build_system_prompt, BOOTSTRAP_MAX_CHARS};
use tempfile::TempDir;

#[test]
fn prompt_contains_all_sections() {
    let ws = make_workspace();
    let tools = vec![("shell", "Run commands"), ("file_read", "Read files")];
    let prompt = build_system_prompt(ws.path(), "test-model", &tools, &[], None, None);

    // Section headers
    assert!(prompt.contains("## Tools"), "missing Tools section");
    assert!(prompt.contains("## Safety"), "missing Safety section");
    assert!(prompt.contains("## Workspace"), "missing Workspace section");
    assert!(
        prompt.contains("## Project Context"),
        "missing Project Context"
    );
    assert!(
        prompt.contains("## Current Date & Time"),
        "missing Date/Time"
    );
    assert!(prompt.contains("## Runtime"), "missing Runtime section");
}

#[test]
fn prompt_injects_tools() {
    let ws = make_workspace();
    let tools = vec![
        ("shell", "Run commands"),
        ("memory_recall", "Search memory"),
    ];
    let prompt = build_system_prompt(ws.path(), "gpt-4o", &tools, &[], None, None);

    assert!(prompt.contains("**shell**"));
    assert!(prompt.contains("Run commands"));
    assert!(prompt.contains("**memory_recall**"));
}

#[test]
fn prompt_injects_safety() {
    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    assert!(prompt.contains("Do not exfiltrate private data"));
    assert!(prompt.contains("Do not run destructive commands"));
    assert!(prompt.contains("Prefer `trash` over `rm`"));
}

#[test]
fn prompt_injects_workspace_files() {
    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    assert!(prompt.contains("### SOUL.md"), "missing SOUL.md header");
    assert!(prompt.contains("Be helpful"), "missing SOUL content");
    assert!(prompt.contains("### IDENTITY.md"), "missing IDENTITY.md");
    assert!(
        prompt.contains("Name: OpenHuman"),
        "missing IDENTITY content"
    );
    assert!(prompt.contains("### USER.md"), "missing USER.md");
    assert!(prompt.contains("### AGENTS.md"), "missing AGENTS.md");
    assert!(prompt.contains("### TOOLS.md"), "missing TOOLS.md");
    // HEARTBEAT.md is intentionally excluded from channel prompts — it's only
    // relevant to the heartbeat worker and causes LLMs to emit spurious
    // "HEARTBEAT_OK" acknowledgments in channel conversations.
    assert!(
        !prompt.contains("### HEARTBEAT.md"),
        "HEARTBEAT.md should not be in channel prompt"
    );
    assert!(prompt.contains("### MEMORY.md"), "missing MEMORY.md");
    assert!(prompt.contains("User likes Rust"), "missing MEMORY content");
}

#[test]
fn prompt_missing_file_markers() {
    let tmp = TempDir::new().unwrap();
    // Empty workspace — no files at all
    let prompt = build_system_prompt(tmp.path(), "model", &[], &[], None, None);

    assert!(prompt.contains("[File not found: SOUL.md]"));
    assert!(prompt.contains("[File not found: AGENTS.md]"));
    assert!(prompt.contains("[File not found: IDENTITY.md]"));
}

#[test]
fn prompt_bootstrap_only_if_exists() {
    let ws = make_workspace();
    // No BOOTSTRAP.md — should not appear
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);
    assert!(
        !prompt.contains("### BOOTSTRAP.md"),
        "BOOTSTRAP.md should not appear when missing"
    );

    // Create BOOTSTRAP.md — should appear
    std::fs::write(ws.path().join("BOOTSTRAP.md"), "# Bootstrap\nFirst run.").unwrap();
    let prompt2 = build_system_prompt(ws.path(), "model", &[], &[], None, None);
    assert!(
        prompt2.contains("### BOOTSTRAP.md"),
        "BOOTSTRAP.md should appear when present"
    );
    assert!(prompt2.contains("First run"));
}

#[test]
fn prompt_no_daily_memory_injection() {
    let ws = make_workspace();
    let memory_dir = ws.path().join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    std::fs::write(
        memory_dir.join(format!("{today}.md")),
        "# Daily\nSome note.",
    )
    .unwrap();

    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    // Daily notes should NOT be in the system prompt (on-demand via tools)
    assert!(
        !prompt.contains("Daily Notes"),
        "daily notes should not be auto-injected"
    );
    assert!(
        !prompt.contains("Some note"),
        "daily content should not be in prompt"
    );
}

#[test]
fn prompt_runtime_metadata() {
    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "claude-sonnet-4", &[], &[], None, None);

    assert!(prompt.contains("Model: claude-sonnet-4"));
    assert!(prompt.contains(&format!("OS: {}", std::env::consts::OS)));
    assert!(prompt.contains("Host:"));
}

#[test]
fn prompt_skills_compact_list() {
    let ws = make_workspace();
    let skills = vec![crate::openhuman::skills::Skill {
        name: "code-review".into(),
        description: "Review code for bugs".into(),
        version: "1.0.0".into(),
        author: None,
        tags: vec![],
        tools: vec![],
        prompts: vec!["Long prompt content that should NOT appear in system prompt".into()],
        location: None,
    }];

    let prompt = build_system_prompt(ws.path(), "model", &[], &skills, None, None);

    assert!(prompt.contains("<available_skills>"), "missing skills XML");
    assert!(prompt.contains("<name>code-review</name>"));
    assert!(prompt.contains("<description>Review code for bugs</description>"));
    assert!(prompt.contains("SKILL.md</location>"));
    assert!(
        prompt.contains("loaded on demand"),
        "should mention on-demand loading"
    );
    // Full prompt content should NOT be dumped
    assert!(!prompt.contains("Long prompt content that should NOT appear"));
}

#[test]
fn prompt_truncation() {
    let ws = make_workspace();
    // Write a file larger than BOOTSTRAP_MAX_CHARS
    let big_content = "x".repeat(BOOTSTRAP_MAX_CHARS + 1000);
    std::fs::write(ws.path().join("AGENTS.md"), &big_content).unwrap();

    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    assert!(
        prompt.contains("truncated at"),
        "large files should be truncated"
    );
    assert!(
        !prompt.contains(&big_content),
        "full content should not appear"
    );
}

#[test]
fn prompt_empty_files_skipped() {
    let ws = make_workspace();
    std::fs::write(ws.path().join("TOOLS.md"), "").unwrap();

    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    // Empty file should not produce a header
    assert!(
        !prompt.contains("### TOOLS.md"),
        "empty files should be skipped"
    );
}

#[test]
fn channel_log_truncation_is_utf8_safe_for_multibyte_text() {
    let msg = "Hello from OpenHuman 🌍. Current status is healthy, and café-style UTF-8 text stays safe in logs.";

    // Reproduces the production crash path where channel logs truncate at 80 chars.
    let result = std::panic::catch_unwind(|| crate::openhuman::util::truncate_with_ellipsis(msg, 80));
    assert!(
        result.is_ok(),
        "truncate_with_ellipsis should never panic on UTF-8"
    );

    let truncated = result.unwrap();
    assert!(!truncated.is_empty());
    assert!(truncated.is_char_boundary(truncated.len()));
}

#[test]
fn prompt_contains_channel_capabilities() {
    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

    assert!(
        prompt.contains("## Channel Capabilities"),
        "missing Channel Capabilities section"
    );
    assert!(
        prompt.contains("running as a Discord bot"),
        "missing Discord context"
    );
    assert!(
        prompt.contains("NEVER repeat, describe, or echo credentials"),
        "missing security instruction"
    );
}

#[test]
fn prompt_workspace_path() {
    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);
    let workspace_path = ws.path().display().to_string();

    assert!(
        prompt.contains(&workspace_path),
        "workspace path missing from prompt"
    );
}
