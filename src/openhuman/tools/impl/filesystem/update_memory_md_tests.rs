use super::*;

fn make_tool(dir: &std::path::Path) -> UpdateMemoryMdTool {
    UpdateMemoryMdTool::new(dir.to_path_buf())
}

#[tokio::test]
async fn append_creates_file_if_missing() {
    let dir = tempfile::tempdir().unwrap();
    let tool = make_tool(dir.path());
    let result = tool
        .execute(json!({
            "file": "MEMORY.md",
            "action": "append",
            "content": "first note"
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{:?}", result.output());
    let text = std::fs::read_to_string(dir.path().join("MEMORY.md")).unwrap();
    assert!(text.contains("first note"));
}

#[tokio::test]
async fn append_adds_to_existing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("MEMORY.md");
    std::fs::write(&path, "existing\n").unwrap();
    let tool = make_tool(dir.path());
    tool.execute(json!({
        "file": "MEMORY.md",
        "action": "append",
        "content": "second note"
    }))
    .await
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("existing"));
    assert!(text.contains("second note"));
}

#[tokio::test]
async fn replace_section_overwrites_body() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("MEMORY.md");
    std::fs::write(&path, "## Lessons\nold body\n## Other\nkept\n").unwrap();
    let tool = make_tool(dir.path());
    tool.execute(json!({
        "file": "MEMORY.md",
        "action": "replace_section",
        "section_title": "Lessons",
        "content": "new body"
    }))
    .await
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("new body"), "new body missing: {text}");
    assert!(
        !text.contains("old body"),
        "old body should be gone: {text}"
    );
    assert!(text.contains("## Other"), "other section missing: {text}");
    assert!(text.contains("kept"), "other section body missing: {text}");
}

#[tokio::test]
async fn replace_section_appends_when_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("SKILL.md");
    std::fs::write(&path, "# Header\n").unwrap();
    let tool = make_tool(dir.path());
    tool.execute(json!({
        "file": "SKILL.md",
        "action": "replace_section",
        "section_title": "New Section",
        "content": "brand new"
    }))
    .await
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("## New Section"), "heading missing: {text}");
    assert!(text.contains("brand new"), "content missing: {text}");
}

#[tokio::test]
async fn replace_section_with_empty_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("MEMORY.md");
    std::fs::write(&path, "## Notes\nold stuff\n## End\ndone\n").unwrap();
    let tool = make_tool(dir.path());
    tool.execute(json!({
        "file": "MEMORY.md",
        "action": "replace_section",
        "section_title": "Notes",
        "content": ""
    }))
    .await
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains("old stuff"),
        "old body should be gone: {text}"
    );
    assert!(text.contains("## End"), "other section missing: {text}");
}

#[tokio::test]
async fn append_to_empty_memory_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("MEMORY.md");
    std::fs::write(&path, "").unwrap();
    let tool = make_tool(dir.path());
    let result = tool
        .execute(json!({
            "file": "MEMORY.md",
            "action": "append",
            "content": "first line"
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "unexpected error: {}", result.output());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("first line"));
}

#[tokio::test]
async fn replace_section_creates_memory_file_if_missing() {
    let dir = tempfile::tempdir().unwrap();
    let tool = make_tool(dir.path());
    let result = tool
        .execute(json!({
            "file": "MEMORY.md",
            "action": "replace_section",
            "section_title": "First",
            "content": "hello"
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "unexpected error: {}", result.output());
    let text = std::fs::read_to_string(dir.path().join("MEMORY.md")).unwrap();
    assert!(text.contains("## First"));
    assert!(text.contains("hello"));
}

#[tokio::test]
async fn rejects_unknown_action() {
    let dir = tempfile::tempdir().unwrap();
    let tool = make_tool(dir.path());
    let result = tool
        .execute(json!({
            "file": "MEMORY.md",
            "action": "delete_all",
            "content": "x"
        }))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn replace_section_missing_section_title_errors() {
    let dir = tempfile::tempdir().unwrap();
    let tool = make_tool(dir.path());
    let result = tool
        .execute(json!({
            "file": "MEMORY.md",
            "action": "replace_section",
            "content": "x"
        }))
        .await;
    // May return Err or Ok with is_error
    match result {
        Ok(r) => assert!(r.is_error),
        Err(_) => {} // also acceptable
    }
}

#[test]
fn tool_name_and_description() {
    let dir = tempfile::tempdir().unwrap();
    let tool = make_tool(dir.path());
    assert_eq!(tool.name(), "update_memory_md");
    assert!(!tool.description().is_empty());
}

#[test]
fn parameters_schema_has_required_fields() {
    let dir = tempfile::tempdir().unwrap();
    let tool = make_tool(dir.path());
    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("file")));
    assert!(required.contains(&json!("action")));
}

#[tokio::test]
async fn rejects_disallowed_file() {
    let dir = tempfile::tempdir().unwrap();
    let tool = make_tool(dir.path());
    let result = tool
        .execute(json!({
            "file": "../../etc/passwd",
            "action": "append",
            "content": "evil"
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("not allowed"));
}
