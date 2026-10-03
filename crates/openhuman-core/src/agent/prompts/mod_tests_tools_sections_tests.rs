use super::*;

#[test]
fn rendered_subagent_system_prompt_is_byte_stable_across_repeat_calls() {
    // KV-cache contract: two spawns of the same sub-agent definition
    // against the same workspace must produce byte-identical system
    // prompts, or the backend's automatic prefix cache busts. This test
    // pins the invariant end-to-end.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_byte_stable_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let opts = SubagentRenderOptions {
        include_identity: true,
        include_safety_preamble: true,
    };

    let first = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are the orchestrator.",
        opts,
        ToolCallFormat::PFormat,
        &[],
    );
    let second = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are the orchestrator.",
        opts,
        ToolCallFormat::PFormat,
        &[],
    );

    assert_eq!(
        first, second,
        "repeat spawns must produce byte-identical prompts"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn sync_workspace_file_updates_hash_and_inject_workspace_file_truncates() {
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_workspace_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();

    sync_workspace_file(&workspace, "SOUL.md");
    let hash_path = workspace.join(".SOUL.md.builtin-hash");
    assert!(workspace.join("SOUL.md").exists());
    assert!(hash_path.exists());
    let original_hash = std::fs::read_to_string(&hash_path).unwrap();

    std::fs::write(workspace.join("SOUL.md"), "user override").unwrap();
    sync_workspace_file(&workspace, "SOUL.md");
    assert_eq!(std::fs::read_to_string(&hash_path).unwrap(), original_hash);
    assert_eq!(
        std::fs::read_to_string(workspace.join("SOUL.md")).unwrap(),
        "user override"
    );

    std::fs::write(
        workspace.join("BIG.md"),
        "x".repeat(BOOTSTRAP_MAX_CHARS + 50),
    )
    .unwrap();
    let mut prompt = String::new();
    inject_workspace_file(&mut prompt, &workspace, "BIG.md");
    assert!(prompt.contains("### BIG.md"));
    assert!(prompt.contains("[... truncated at"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn prompt_tool_constructors_and_unknown_default_file() {
    let plain = PromptTool::new("shell", "run commands");
    assert_eq!(plain.name, "shell");
    assert!(plain.parameters_schema.is_none());

    let with_schema =
        PromptTool::with_schema("http_request", "fetch data", "{\"type\":\"object\"}".into());
    assert_eq!(
        with_schema.parameters_schema.as_deref(),
        Some("{\"type\":\"object\"}")
    );
    assert_eq!(default_workspace_file_content("missing"), "");
}

// ─── ToolsSection native-skip tests ──────────────────────────────────────────

#[test]
fn deferred_browser_prompt_has_only_discovery_hint_in_native_and_text_modes() {
    let tools = vec![
        PromptTool::owned(
            "browser".into(),
            "browser full schema".into(),
            r#"{"type":"object","properties":{"secret_browser_action":{"type":"string"}}}"#.into(),
        ),
        PromptTool::owned(
            "browser_open".into(),
            "browser open schema".into(),
            r#"{"type":"object","properties":{"secret_open_url":{"type":"string"}}}"#.into(),
        ),
        PromptTool::owned(
            "tool_search".into(),
            "Find tools".into(),
            r#"{"type":"object"}"#.into(),
        ),
    ];
    let visible = std::collections::HashSet::from(["tool_search".to_string()]);
    for format in [ToolCallFormat::Native, ToolCallFormat::Python] {
        let ctx = PromptContext {
            workspace_dir: Path::new("/tmp"),
            model_name: "test-model",
            agent_id: "",
            tools: &tools,
            workflows: &[],
            dispatcher_instructions: "",
            visible_tool_names: &visible,
            tool_call_format: format,
            connected_integrations: &[],
            connected_identities_md: String::new(),
            user_identity: None,
            personality_roster: vec![],
            agents_md_global: None,
            agents_md_local: None,
        };
        let rendered = ToolsSection.build(&ctx).unwrap();
        assert!(rendered.contains("For website tasks, use tool_search to find browser tools."));
        assert!(!rendered.contains("secret_browser_action"));
        assert!(!rendered.contains("secret_open_url"));
    }
}

#[test]
fn tools_section_empty_for_native() {
    // Native function-calling: the provider sends full JSON schemas in the
    // API request — repeating them in the system prompt is pure token bloat.
    // ToolsSection must return an empty string for Native mode.
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
        workflows: &[],
        dispatcher_instructions: "",
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::Native,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        user_identity: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    };
    let out = ToolsSection.build(&ctx).unwrap();
    assert!(
        out.is_empty(),
        "Native mode should produce empty ToolsSection, got: {out:?}"
    );
}

#[test]
fn tools_section_nonempty_for_pformat() {
    // PFormat is a text-driven format — the model discovers tools by reading
    // the prose `## Tools` section. It must be non-empty.
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
        workflows: &[],
        dispatcher_instructions: "",
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        user_identity: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    };
    let out = ToolsSection.build(&ctx).unwrap();
    assert!(
        out.contains("## Tools"),
        "PFormat should render tool catalogue header, got: {out:?}"
    );
}

#[test]
fn tools_section_native_with_dispatcher_instructions_returns_instructions() {
    // Native mode must still include non-empty dispatcher_instructions
    // (e.g. the "## Tool Use Protocol" block from NativeDialect) so
    // the model receives behavioural guidance even though the tool catalogue
    // itself is omitted.
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
        workflows: &[],
        dispatcher_instructions: "## Tool Use Protocol\n\nUse native tool calling.",
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::Native,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        user_identity: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    };
    let out = ToolsSection.build(&ctx).unwrap();
    assert!(
        out.contains("## Tool Use Protocol"),
        "Native mode with non-empty dispatcher_instructions must include them, got: {out:?}"
    );
    assert!(
        !out.contains("## Tools"),
        "Native mode must not include the tool catalogue header, got: {out:?}"
    );
}

#[test]
fn agents_md_section_empty_when_both_layers_absent() {
    let ctx = agents_md_ctx(None, None);
    let out = AgentsInstructionsSection.build(&ctx).unwrap();
    assert!(
        out.trim().is_empty(),
        "section must be empty when no AGENTS.md content is present, got: {out:?}"
    );
}

#[test]
fn agents_md_section_renders_global_only() {
    let ctx = agents_md_ctx(Some("workspace rule one".into()), None);
    let out = AgentsInstructionsSection.build(&ctx).unwrap();
    assert!(out.contains("## Project instructions (AGENTS.md)"));
    assert!(out.contains("AGENTS.md (workspace)"));
    assert!(out.contains("workspace rule one"));
    assert!(
        !out.contains("AGENTS.md (project)"),
        "no project layer should be rendered, got: {out}"
    );
}

#[test]
fn agents_md_section_renders_local_only() {
    let ctx = agents_md_ctx(None, Some("project rule two".into()));
    let out = AgentsInstructionsSection.build(&ctx).unwrap();
    assert!(out.contains("## Project instructions (AGENTS.md)"));
    assert!(out.contains("AGENTS.md (project)"));
    assert!(out.contains("project rule two"));
}

#[test]
fn agents_md_section_layers_global_before_local() {
    let ctx = agents_md_ctx(Some("GLOBAL_MARKER".into()), Some("LOCAL_MARKER".into()));
    let out = AgentsInstructionsSection.build(&ctx).unwrap();
    let g = out.find("GLOBAL_MARKER").expect("global present");
    let l = out.find("LOCAL_MARKER").expect("local present");
    assert!(
        g < l,
        "global layer must render before local layer, got: {out}"
    );
    // Both sub-headings present.
    assert!(out.contains("AGENTS.md (workspace)"));
    assert!(out.contains("AGENTS.md (project)"));
}
