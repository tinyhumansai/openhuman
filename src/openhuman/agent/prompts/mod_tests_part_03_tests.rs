use super::*;

#[test]
fn profile_md_and_memory_md_are_capped_at_user_file_max_chars() {
    // Both PROFILE.md and MEMORY.md are user-specific files that can
    // grow over time. Injection caps them at USER_FILE_MAX_CHARS
    // (~1000 tokens each) so the system prompt footprint stays
    // bounded. Test both files at once to pin the shared budget.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_user_cap_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let big = "x".repeat(USER_FILE_MAX_CHARS + 500);
    std::fs::write(workspace.join("PROFILE.md"), &big).unwrap();
    std::fs::write(workspace.join("MEMORY.md"), &big).unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are the orchestrator.",
        SubagentRenderOptions {
            include_identity: false,
            include_safety_preamble: false,
            include_profile: true,
            include_memory_md: true,
        },
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(rendered.contains("### PROFILE.md"));
    assert!(rendered.contains("### MEMORY.md"));
    // Each file gets its own truncation marker mentioning the cap.
    let marker = format!("[... truncated at {USER_FILE_MAX_CHARS} chars");
    assert_eq!(
        rendered.matches(marker.as_str()).count(),
        2,
        "both PROFILE.md and MEMORY.md must emit the truncation marker at \
         USER_FILE_MAX_CHARS — found:\n{rendered}"
    );
    // Sanity-check the cap is genuinely tighter than the bootstrap cap.
    assert!(USER_FILE_MAX_CHARS < BOOTSTRAP_MAX_CHARS);

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn rendered_subagent_system_prompt_is_byte_stable_across_repeat_calls() {
    // KV-cache contract: two spawns of the same sub-agent definition
    // against the same workspace must produce byte-identical system
    // prompts. If PROFILE.md or MEMORY.md are re-read with a
    // different-typed truncation path, or if either cap drifts, the
    // bytes differ and the backend's automatic prefix cache busts.
    // This test pins the invariant end-to-end.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_byte_stable_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("PROFILE.md"), "# User Profile\nJane Doe").unwrap();
    std::fs::write(workspace.join("MEMORY.md"), "# Memory\nRecent: shipped v1").unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let opts = SubagentRenderOptions {
        include_identity: false,
        include_safety_preamble: false,
        include_profile: true,
        include_memory_md: true,
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
fn for_subagent_builder_injects_user_files_even_when_identity_omitted() {
    // Regression pin for the review finding: the runtime Tauri chat
    // path spins welcome/trigger_* via `Agent::from_config_for_agent`
    // → `SystemPromptBuilder::for_subagent(body, omit_identity=true, …)`,
    // which deliberately drops `IdentitySection`. Before
    // `UserFilesSection` existed, our PROFILE/MEMORY injection lived
    // inside `IdentitySection::build` and got dropped along with it,
    // so the first Tauri turn never saw the user's onboarding output
    // even though the subagent_runner path and the debug dumper did.
    //
    // This test exercises the exact builder call-site the runtime
    // uses for welcome (`omit_identity = true`, both user-file flags
    // opted in via PromptContext) and pins that the rendered prompt
    // contains both files.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_for_subagent_user_files_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("PROFILE.md"),
        "# User Profile\nJane Doe — crypto trader in PST.",
    )
    .unwrap();
    std::fs::write(
        workspace.join("MEMORY.md"),
        "# Long-term memory\nShipped v1 last sprint; prefers terse Rust.",
    )
    .unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = PromptContext {
        workspace_dir: &workspace,
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
        workflows: &[],
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: true,
        include_memory_md: true,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    };

    // Test a narrow-agent runtime path:
    // `SystemPromptBuilder::for_subagent(body, omit_identity=true, …)`.
    let builder = SystemPromptBuilder::for_subagent(
        "You are a specialist agent.".into(),
        true, // omit_identity  — drops SOUL/IDENTITY preamble
        true, // omit_safety_preamble
    );
    let rendered = builder.build(&ctx).unwrap();

    assert!(
        !rendered.contains("## Project Context"),
        "identity preamble must still be suppressed when omit_identity=true"
    );
    assert!(
        rendered.contains("### PROFILE.md") && rendered.contains("Jane Doe"),
        "narrow runtime path must inject PROFILE.md despite omit_identity=true, got:\n{rendered}"
    );
    assert!(
        rendered.contains("### MEMORY.md") && rendered.contains("terse Rust"),
        "narrow runtime path must inject MEMORY.md despite omit_identity=true, got:\n{rendered}"
    );

    // Mirror the narrow-specialist runtime path (code_executor,
    // critic, …): both flags off → user files must stay out.
    let ctx_narrow = PromptContext {
        workspace_dir: &workspace,
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
        workflows: &[],
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    };
    let narrow = builder.build(&ctx_narrow).unwrap();
    assert!(
        !narrow.contains("### PROFILE.md") && !narrow.contains("### MEMORY.md"),
        "narrow specialist runtime path must NOT leak user files, got:\n{narrow}"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn memory_md_injection_is_framed_as_background_not_prior_chat() {
    // GH-4745 regression: MEMORY.md is durable cross-session memory. Without
    // a frame, a relevant curated observation reads to the model as prior
    // *in-thread* conversation, so on a brand-new thread it opens with
    // "already covered this in a previous chat" and shortcuts its answer.
    // Pin that the rendered prompt frames the block as background memory and
    // that the guardrail precedes the injected `### MEMORY.md` heading.
    //
    // `tempfile::tempdir()` cleans up via `Drop` even when an assertion
    // below panics — a bare `remove_dir_all` at the tail would leak the
    // dir exactly on the failing run we most want to inspect.
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(
        workspace.path().join("MEMORY.md"),
        "# Long-term memory\nReviewed `def f(x)` last week; user prefers terse notes.",
    )
    .unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = memory_framing_ctx(workspace.path(), &prompt_tools);

    let rendered = UserFilesSection.build(&ctx).unwrap();

    assert!(
        rendered.contains("### MEMORY.md") && rendered.contains("terse notes"),
        "MEMORY.md must still be injected, got:\n{rendered}"
    );
    assert!(
        rendered.contains("background — not this conversation"),
        "MEMORY.md must be framed as durable background memory, got:\n{rendered}"
    );
    assert!(
        rendered.contains("already covered this in a previous chat"),
        "framing must explicitly forbid asserting prior-chat continuity, got:\n{rendered}"
    );
    let frame_at = rendered.find("background — not this conversation").unwrap();
    let heading_at = rendered.find("### MEMORY.md").unwrap();
    assert!(
        frame_at < heading_at,
        "the guardrail note must precede the MEMORY.md block, got:\n{rendered}"
    );
}

#[test]
fn memory_md_framing_absent_when_no_memory_content() {
    // The frame must never appear on its own: when MEMORY.md is missing/empty
    // (a genuinely fresh workspace) there is nothing to scope, so emitting a
    // dangling "background memory" note would itself imply phantom history.
    let workspace = tempfile::tempdir().unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = memory_framing_ctx(workspace.path(), &prompt_tools);

    let rendered = UserFilesSection.build(&ctx).unwrap();
    assert!(
        !rendered.contains("background — not this conversation"),
        "no MEMORY.md content → no dangling framing note, got:\n{rendered}"
    );
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
fn prompt_tool_constructors_and_user_memory_skip_empty_bodies() {
    let plain = PromptTool::new("shell", "run commands");
    assert_eq!(plain.name, "shell");
    assert!(plain.parameters_schema.is_none());

    let with_schema =
        PromptTool::with_schema("http_request", "fetch data", "{\"type\":\"object\"}".into());
    assert_eq!(
        with_schema.parameters_schema.as_deref(),
        Some("{\"type\":\"object\"}")
    );

    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "model",
        agent_id: "",
        tools: &[],
        workflows: &[],
        dispatcher_instructions: "",
        learned: LearnedContextData {
            tree_root_summaries: vec![ns_summary("user", "kept"), ns_summary("empty", "   ")],
            ..Default::default()
        },
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    };
    let rendered = UserMemorySection.build(&ctx).unwrap();
    assert!(rendered.contains("### user"));
    assert!(!rendered.contains("### empty"));
    assert_eq!(default_workspace_file_content("missing"), "");
}

#[test]
fn user_reflections_section_renders_bullets_with_priority_preamble() {
    let ctx = ctx_with_learned(LearnedContextData {
        reflections: vec![
            "Going forward I want concise replies".into(),
            "I realized I prefer Rust over TypeScript".into(),
        ],
        ..Default::default()
    });
    let rendered = UserReflectionsSection.build(&ctx).unwrap();
    assert!(rendered.starts_with("## User Reflections\n\n"));
    assert!(
        rendered.contains("higher-priority"),
        "preamble must signal that reflections outrank generic memory"
    );
    assert!(rendered.contains("- Going forward I want concise replies"));
    assert!(rendered.contains("- I realized I prefer Rust over TypeScript"));
}

#[test]
fn user_reflections_section_returns_empty_without_entries() {
    let ctx = ctx_with_learned(LearnedContextData::default());
    assert!(UserReflectionsSection.build(&ctx).unwrap().is_empty());
}

#[test]
fn user_reflections_section_skips_blank_entries() {
    let ctx = ctx_with_learned(LearnedContextData {
        reflections: vec!["   ".into(), "Real reflection".into(), "".into()],
        ..Default::default()
    });
    let rendered = UserReflectionsSection.build(&ctx).unwrap();
    assert!(rendered.contains("- Real reflection"));
    // Bullet count should match the non-blank entry count.
    assert_eq!(rendered.matches("\n- ").count(), 1);
}

#[test]
fn render_user_reflections_helper_matches_section_output() {
    let ctx = ctx_with_learned(LearnedContextData {
        reflections: vec!["x".into()],
        ..Default::default()
    });
    let via_section = UserReflectionsSection.build(&ctx).unwrap();
    let via_helper = render_user_reflections(&ctx).unwrap();
    assert_eq!(via_section, via_helper);
}

#[test]
fn insert_section_before_places_section_ahead_of_named_target() {
    // Reflections must rank ahead of generic memory in builders that
    // already include `UserMemorySection` (the `with_defaults` chain).
    // Verify the helper inserts at the correct index instead of
    // tail-appending.
    let builder = SystemPromptBuilder::with_defaults()
        .insert_section_before("user_memory", Box::new(UserReflectionsSection));
    let names: Vec<&str> = builder.sections.iter().map(|s| s.name()).collect();
    let r_idx = names
        .iter()
        .position(|n| *n == "user_reflections")
        .expect("user_reflections section");
    let m_idx = names
        .iter()
        .position(|n| *n == "user_memory")
        .expect("user_memory section");
    assert!(
        r_idx < m_idx,
        "insert_section_before should place the new section ahead of its target, got order {names:?}"
    );
}

#[test]
fn insert_section_before_falls_back_to_append_when_target_missing() {
    // Dynamic / sub-agent builders do not include a `user_memory`
    // section. The helper should still land the new section so the
    // caller's wiring stays loop-free, just at the tail.
    let builder = SystemPromptBuilder::default()
        .add_section(Box::new(SafetySection))
        .insert_section_before("user_memory", Box::new(UserReflectionsSection));
    let names: Vec<&str> = builder.sections.iter().map(|s| s.name()).collect();
    assert_eq!(names.last(), Some(&"user_reflections"));
    assert_eq!(names.len(), 2);
}

#[test]
fn user_reflections_render_above_user_memory_when_both_present() {
    // Acceptance criterion: reflections rank above generic
    // tree summaries — verify by composing the same way the runtime
    // does (UserReflectionsSection appended ahead of any
    // UserMemorySection content).
    let ctx = ctx_with_learned(LearnedContextData {
        reflections: vec!["I want terse answers".into()],
        tree_root_summaries: vec![ns_summary("user", "Generic summary")],
        ..Default::default()
    });
    let reflections = UserReflectionsSection.build(&ctx).unwrap();
    let memory = UserMemorySection.build(&ctx).unwrap();
    let combined = format!("{reflections}{memory}");
    let r_idx = combined
        .find("## User Reflections")
        .expect("reflections heading");
    let m_idx = combined.find("## User Memory").expect("memory heading");
    assert!(
        r_idx < m_idx,
        "reflections must render before user-memory block"
    );
}

// ─── ToolsSection native-skip tests ──────────────────────────────────────────

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
        learned: LearnedContextData::default(),
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::Native,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
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
        learned: LearnedContextData::default(),
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
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
    // (e.g. the "## Tool Use Protocol" block from NativeToolDispatcher) so
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
        learned: LearnedContextData::default(),
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::Native,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
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
