use super::common::make_workspace;
use crate::agent::context::channels_prompt::build_system_prompt;

/// `build_system_prompt` loads OpenClaw markdown identity files from the
/// workspace and inlines their contents into the Project Context section.
#[test]
fn openclaw_loads_workspace_markdown_files() {
    let ws = make_workspace();
    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, Some("Discord"));

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
    // PROFILE.md and MEMORY.md were v1 memory artefacts. Memory v2 replaces
    // both with `context.md`, injected as the first user message of a new
    // session, so they are never inlined even when a workspace still has them.
    assert!(
        !prompt.contains("Name: Test User"),
        "PROFILE.md must not be inlined"
    );
    assert!(
        !prompt.contains("User likes Rust"),
        "MEMORY.md must not be inlined"
    );
}
