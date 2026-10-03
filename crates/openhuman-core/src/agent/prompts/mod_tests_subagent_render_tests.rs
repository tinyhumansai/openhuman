use super::*;

#[test]
fn render_subagent_system_prompt_renders_workspace_tail() {
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_subagent_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a focused sub-agent.",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(rendered.contains("## Workspace"));
    assert!(rendered.contains("## Runtime"));
    // Grounding contract is appended even by the narrow (index-based)
    // sub-agent renderer — same source const, so it can never drift from
    // `GroundingSection` / the central `build()` append.
    assert!(rendered.contains("## Grounding and tool use"));
    assert!(rendered.contains("Your tools are exactly the ones you have been given for this turn"));
    assert!(rendered.contains("Preserve numeric evidence exactly"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn subagent_prompt_defaults_to_json_and_omits_protocol_without_tools() {
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_default_dialect_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let default_rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a focused sub-agent.",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::default(),
        &[],
    );
    assert!(!default_rendered.contains("def test_tool() -> str"));
    assert!(default_rendered.contains("test_tool"));
    assert!(!default_rendered.contains("test_tool[]"));

    let no_tools = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[],
        &[],
        &[],
        "You are a focused sub-agent.",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::Json,
        &[],
    );
    assert!(!no_tools.contains("## Tools"));
    assert!(!no_tools.contains("## Tool Use Protocol"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn subagent_render_options_invert_definition_flags() {
    // (omit_identity, omit_safety_preamble)
    let options = SubagentRenderOptions::from_definition_flags(true, false);
    assert!(!options.include_identity);
    assert!(options.include_safety_preamble);
    let narrow = SubagentRenderOptions::narrow();
    assert!(!narrow.include_identity);
    assert!(!narrow.include_safety_preamble);
}

#[test]
fn render_subagent_system_prompt_honors_identity_safety_and_skills_flags() {
    let workspace =
        std::env::temp_dir().join(format!("openhuman_prompt_opts_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("SOUL.md"), "# Soul\nContext").unwrap();
    std::fs::write(workspace.join("IDENTITY.md"), "# Identity\nContext").unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt_with_format(
        &workspace,
        "reasoning-v1",
        &[0],
        &tools,
        &[],
        "You are a specialist.",
        SubagentRenderOptions {
            include_identity: true,
            include_safety_preamble: true,
        },
        ToolCallFormat::Json,
        &[],
        None,
        None,
    );

    assert!(rendered.contains("## Project Context"));
    assert!(rendered.contains("### SOUL.md"));
    assert!(rendered.contains("## Safety"));
    // Json is a prompt-driven format (the model wraps JSON tool
    // calls in `<tool_call>` tags); it does NOT use the provider's
    // native function-calling channel. So the prose tool catalogue
    // MUST still be rendered for Json, with each tool's compact
    // argument signature so the model knows what to emit.
    // Only `ToolCallFormat::Native` gets the section omitted (see
    // the `native` branch below and the `!matches!(…, Native)`
    // guard in the renderer).
    assert!(rendered.contains("### Available Tools"));
    assert!(rendered.contains("**test_tool**"));
    assert!(rendered.contains("Arguments: `object`"));

    let native = render_subagent_system_prompt_with_format(
        &workspace,
        "reasoning-v1",
        &[0],
        &tools,
        &[],
        "You are a specialist.",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::Native,
        &[],
        None,
        None,
    );
    assert!(native.contains("through native tool-calling."));
    assert!(!native.contains("## Safety"));
    // Native is the only format where the prose `## Tools` section
    // is intentionally omitted — schemas travel through the
    // provider's `tools` field instead. Regression guard against
    // the ~54k-token schema duplication from the #447 PR.
    assert!(!native.contains("\n## Tools\n"));
    assert!(!native.contains("Parameters:"));

    let _ = std::fs::remove_dir_all(workspace);
}

/// Render a sub-agent prompt over a scratch workspace holding `files`, with the
/// PFormat tool-call format and the single `TestTool`.
fn render_with_files(files: &[(&str, &str)], options: SubagentRenderOptions) -> String {
    let workspace = tempfile::tempdir().expect("tempdir");
    for (name, body) in files {
        std::fs::write(workspace.path().join(name), body).unwrap();
    }
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    render_subagent_system_prompt(
        workspace.path(),
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a specialist agent.",
        options,
        ToolCallFormat::PFormat,
        &[],
    )
}

#[test]
fn subagent_identity_flag_gates_bootstrap_files_and_never_injects_v1_memory_files() {
    let files = [
        (
            "SOUL.md",
            "# Soul
ctx",
        ),
        (
            "IDENTITY.md",
            "# Identity
ctx",
        ),
        (
            "PROFILE.md",
            "# User Profile
Name: Jane Doe",
        ),
        (
            "MEMORY.md",
            "# Long-term memory
User prefers terse Rust answers.",
        ),
    ];
    let with_identity = render_with_files(
        &files,
        SubagentRenderOptions::from_definition_flags(false, true),
    );
    assert!(with_identity.contains("### SOUL.md"), "{with_identity}");
    assert!(with_identity.contains("### IDENTITY.md"), "{with_identity}");

    let narrow = render_with_files(&files, SubagentRenderOptions::narrow());
    assert!(!narrow.contains("## Project Context"), "{narrow}");

    for rendered in [&with_identity, &narrow] {
        assert!(!rendered.contains("PROFILE.md") && !rendered.contains("Jane Doe"));
        assert!(!rendered.contains("MEMORY.md") && !rendered.contains("terse Rust"));
    }
}

#[test]
fn render_subagent_system_prompt_code_formats_list_signatures_and_protocol() {
    let workspace =
        std::env::temp_dir().join(format!("openhuman_prompt_code_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    for (format, signature, example) in [
        (
            ToolCallFormat::Python,
            "def test_tool() -> str  # tool desc",
            "read_file(path=\"src/main.rs\", limit=20)",
        ),
        (
            ToolCallFormat::TypeScript,
            "function test_tool(): string;  // tool desc",
            "read_file({path: \"src/main.rs\", limit: 20})",
        ),
    ] {
        let rendered = render_subagent_system_prompt_with_format(
            &workspace,
            "reasoning-v1",
            &[0],
            &tools,
            &[],
            "You are a specialist.",
            SubagentRenderOptions::narrow(),
            format,
            &[],
            None,
            None,
        );
        // A prompt-driven format: the child must be told which tools exist,
        // as signatures, and how to call them.
        assert!(rendered.contains("## Tools\n"), "{format:?}:\n{rendered}");
        assert!(rendered.contains(signature), "{format:?}:\n{rendered}");
        assert!(
            rendered.contains("## Tool Use Protocol"),
            "{format:?}:\n{rendered}"
        );
        assert!(rendered.contains(example), "{format:?}:\n{rendered}");
        assert!(!rendered.contains("Parameters:"), "{format:?}:\n{rendered}");
        assert!(!rendered.contains("Call as:"), "{format:?}:\n{rendered}");
        assert!(
            rendered.contains("parent agent will weave it back"),
            "{format:?}:\n{rendered}"
        );
    }

    let _ = std::fs::remove_dir_all(workspace);
}
