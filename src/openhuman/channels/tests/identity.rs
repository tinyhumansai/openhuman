use super::super::prompt::build_system_prompt;
use super::common::make_workspace;

/// `build_system_prompt` loads OpenClaw markdown identity files from the
/// workspace and inlines their contents into the Project Context section.
#[test]
fn openclaw_loads_workspace_markdown_files() {
    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None);

    // Project Context section header is present.
    assert!(
        prompt.contains("## Project Context"),
        "missing Project Context header"
    );

    // Each bundled identity file is inlined (content from make_workspace).
    assert!(
        prompt.contains("Be helpful"),
        "SOUL.md content should be inlined"
    );
    assert!(
        prompt.contains("Name: OpenHuman"),
        "IDENTITY.md content should be inlined"
    );
    assert!(
        prompt.contains("Name: Test User"),
        "USER.md content should be inlined"
    );
    // MEMORY.md is optional (archivist-written). When present it should inline.
    assert!(
        prompt.contains("User likes Rust"),
        "MEMORY.md content should be inlined when present"
    );
}
