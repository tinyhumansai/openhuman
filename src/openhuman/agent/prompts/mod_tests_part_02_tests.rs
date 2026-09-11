use super::*;

#[test]
fn user_memory_section_returns_empty_when_no_summaries() {
    // Empty learned context → section returns empty string and is
    // skipped by the prompt builder, so the cache boundary stays
    // exactly where it was for workspaces with no tree summaries.
    let learned = LearnedContextData::default();
    let prompt_tools: Vec<PromptTool<'_>> = Vec::new();
    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
        workflows: &[],
        dispatcher_instructions: "",
        learned,
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
    assert!(rendered.is_empty());
}

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
    assert!(rendered.contains("Your tools are exactly the ones listed in this prompt"));
    assert!(rendered.contains("Preserve numeric evidence exactly"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn subagent_render_options_invert_definition_flags() {
    // (omit_identity, omit_safety_preamble,
    //  omit_profile, omit_memory_md)
    let options = SubagentRenderOptions::from_definition_flags(true, false, false, false);
    assert!(!options.include_identity);
    assert!(options.include_safety_preamble);
    assert!(options.include_profile);
    assert!(options.include_memory_md);
    let narrow = SubagentRenderOptions::narrow();
    let default = SubagentRenderOptions::default();
    assert_eq!(narrow.include_identity, default.include_identity);
    assert_eq!(
        narrow.include_safety_preamble,
        default.include_safety_preamble
    );
    assert_eq!(narrow.include_profile, default.include_profile);
    assert_eq!(narrow.include_memory_md, default.include_memory_md);
    // Narrow default = every flag off, including both user files.
    assert!(!narrow.include_profile);
    assert!(!narrow.include_memory_md);
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
            include_profile: false,
            include_memory_md: false,
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
    // native function-calling channel. So the prose `## Tools`
    // section MUST still be rendered for Json, with each tool's
    // parameter schema inline so the model knows what to emit.
    // Only `ToolCallFormat::Native` gets the section omitted (see
    // the `native` branch below and the `!matches!(…, Native)`
    // guard in the renderer).
    assert!(rendered.contains("## Tools"));
    assert!(rendered.contains("Parameters:"));
    assert!(rendered.contains("\"type\""));

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
    assert!(native.contains("native tool-calling output"));
    assert!(!native.contains("## Safety"));
    // Native is the only format where the prose `## Tools` section
    // is intentionally omitted — schemas travel through the
    // provider's `tools` field instead. Regression guard against
    // the ~54k-token schema duplication from the #447 PR.
    assert!(!native.contains("\n## Tools\n"));
    assert!(!native.contains("Parameters:"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn render_subagent_system_prompt_injects_profile_md_even_when_identity_omitted() {
    // Regression: an agent with `omit_identity = true` drops the SOUL/IDENTITY
    // preamble but still needs PROFILE.md if `include_profile = true`.
    // PROFILE.md is gated on its own flag so agents can opt in without
    // pulling SOUL/IDENTITY back in.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_profile_nosoul_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("SOUL.md"), "# Soul\nShould be hidden").unwrap();
    std::fs::write(
        workspace.join("IDENTITY.md"),
        "# Identity\nShould be hidden",
    )
    .unwrap();
    std::fs::write(
        workspace.join("PROFILE.md"),
        "# User Profile\nName: Jane Doe\nRole: Data scientist",
    )
    .unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a specialist agent.",
        SubagentRenderOptions {
            include_identity: false,
            include_safety_preamble: false,
            include_profile: true,
            include_memory_md: false,
        },
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        rendered.contains("### PROFILE.md"),
        "PROFILE.md header must appear when include_profile=true, got:\n{rendered}"
    );
    assert!(
        rendered.contains("Jane Doe"),
        "PROFILE.md body must be injected when include_profile=true, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("## Project Context"),
        "identity preamble must still be suppressed when include_identity=false"
    );
    assert!(
        !rendered.contains("### SOUL.md") && !rendered.contains("### IDENTITY.md"),
        "SOUL/IDENTITY must still be suppressed when include_identity=false"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn render_subagent_system_prompt_skips_profile_md_when_include_profile_false() {
    // Mirror of the opt-in regression above: narrow specialists
    // (planner, code_executor, critic, …) set `omit_profile = true`
    // and must NOT see PROFILE.md even when the file is on disk —
    // otherwise every sub-agent pays the token cost of onboarding
    // enrichment output that is irrelevant to their task.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_profile_opt_out_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("PROFILE.md"),
        "# User Profile\nName: Jane Doe\nRole: Data scientist",
    )
    .unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a narrow specialist.",
        SubagentRenderOptions::narrow(), // include_profile defaults to false
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        !rendered.contains("### PROFILE.md"),
        "PROFILE.md must NOT appear when include_profile=false, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("Jane Doe"),
        "PROFILE.md body must NOT be leaked when include_profile=false"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn render_subagent_system_prompt_frames_memory_md_as_background() {
    // GH-4745 regression for the sub-agent path: Inline/File sub-agents inject
    // MEMORY.md through `render_subagent_system_prompt`, a separate renderer
    // from `UserFilesSection`. It must share the same background-memory frame,
    // otherwise a fresh thread reads the bare `### MEMORY.md` block as prior
    // in-thread conversation and asserts continuity that isn't there.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_subagent_memory_framing_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("MEMORY.md"),
        "# Long-term memory\nReviewed `def f(x)` last week; user prefers terse notes.",
    )
    .unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a specialist agent.",
        SubagentRenderOptions {
            include_identity: false,
            include_safety_preamble: false,
            include_profile: false,
            include_memory_md: true,
        },
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        rendered.contains("### MEMORY.md") && rendered.contains("terse notes"),
        "MEMORY.md must still be injected in the sub-agent path, got:\n{rendered}"
    );
    assert!(
        rendered.contains("background — not this conversation"),
        "sub-agent MEMORY.md must be framed as durable background memory, got:\n{rendered}"
    );
    let frame_at = rendered.find("background — not this conversation").unwrap();
    let heading_at = rendered.find("### MEMORY.md").unwrap();
    assert!(
        frame_at < heading_at,
        "the guardrail note must precede the MEMORY.md block, got:\n{rendered}"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn render_subagent_system_prompt_omits_memory_framing_when_no_memory_content() {
    // Companion to the framing test: with `include_memory_md = true` but no
    // MEMORY.md on disk (a genuinely fresh workspace) the dangling frame must
    // NOT appear — emitting a "background memory" note pointing at nothing
    // would itself imply phantom history.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_subagent_memory_noframe_{}",
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
        "You are a specialist agent.",
        SubagentRenderOptions {
            include_identity: false,
            include_safety_preamble: false,
            include_profile: false,
            include_memory_md: true,
        },
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        !rendered.contains("background — not this conversation"),
        "no MEMORY.md content → no dangling framing note in sub-agent path, got:\n{rendered}"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn render_subagent_system_prompt_injects_profile_md_when_identity_included() {
    // When identity is on, PROFILE.md must still be injected alongside
    // SOUL/IDENTITY — the split must not regress the non-welcome path.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_profile_with_identity_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("SOUL.md"), "# Soul\nctx").unwrap();
    std::fs::write(workspace.join("IDENTITY.md"), "# Identity\nctx").unwrap();
    std::fs::write(workspace.join("PROFILE.md"), "# User Profile\nhello").unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a specialist.",
        SubagentRenderOptions {
            include_identity: true,
            include_safety_preamble: false,
            include_profile: true,
            include_memory_md: false,
        },
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(rendered.contains("## Project Context"));
    assert!(rendered.contains("### SOUL.md"));
    assert!(rendered.contains("### IDENTITY.md"));
    assert!(rendered.contains("### PROFILE.md"));
    assert!(rendered.contains("hello"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn render_subagent_system_prompt_silently_skips_missing_profile_md() {
    // Pre-onboarding workspaces have no PROFILE.md. The renderer must
    // not emit a noisy "[File not found: PROFILE.md]" placeholder or
    // an orphan "### PROFILE.md" header — the subagent prompt stays
    // focused on tools.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_profile_missing_{}",
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
        "You are a specialist agent.",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        !rendered.contains("### PROFILE.md"),
        "empty/missing PROFILE.md should not emit a header, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("[File not found: PROFILE.md]"),
        "missing PROFILE.md should be silent, not a noisy placeholder"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn narrow_agent_with_omit_identity_still_loads_profile_md() {
    // Verify that an agent configured with omit_identity=true/
    // omit_safety_preamble=true/omit_profile=false still gets PROFILE.md injected.
    // This exercises the SubagentRenderOptions::from_definition_flags path for agents
    // that want PROFILE.md without the full SOUL/IDENTITY preamble.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_narrow_agent_flags_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("PROFILE.md"),
        "# User Profile\nTimezone: PST\nRole: Crypto trader",
    )
    .unwrap();

    let options = SubagentRenderOptions::from_definition_flags(
        true,  // omit_identity
        true,  // omit_safety_preamble
        false, // omit_profile   — opts IN to PROFILE.md
        false, // omit_memory_md — opts IN to MEMORY.md too
    );

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "# Specialist Agent\n\nYou are a specialist.",
        options,
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        rendered.contains("### PROFILE.md"),
        "agent with omit_profile=false must load PROFILE.md, got:\n{rendered}"
    );
    assert!(
        rendered.contains("Crypto trader"),
        "PROFILE.md body must reach the agent prompt"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn narrow_subagent_definition_flags_skip_profile_md() {
    // Inverse of `welcome_agent_definition_flags_still_load_profile_md`:
    // a narrow specialist (e.g. `code_executor`, `critic`) leaves
    // `omit_profile` at its default `true`. PROFILE.md must NOT be
    // injected even when present on disk — the narrow runner is
    // task-focused and should not pay the token cost.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_narrow_flags_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("PROFILE.md"),
        "# User Profile\nTimezone: PST\nRole: Crypto trader",
    )
    .unwrap();

    // Mirrors e.g. `critic/agent.toml` — all omit_* default-true.
    let options = SubagentRenderOptions::from_definition_flags(true, true, true, true);

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a narrow specialist.",
        options,
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        !rendered.contains("### PROFILE.md"),
        "narrow specialist (omit_profile=true) must NOT load PROFILE.md, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("Crypto trader"),
        "narrow specialist must not leak PROFILE.md body"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn render_subagent_system_prompt_injects_memory_md_when_enabled() {
    // Opt-in agents with `omit_memory_md = false` must see MEMORY.md
    // (archivist-curated long-term memory) in their rendered prompt.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_memory_on_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("MEMORY.md"),
        "# Long-term memory\nUser prefers terse Rust answers.",
    )
    .unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a specialist agent.",
        SubagentRenderOptions {
            include_identity: false,
            include_safety_preamble: false,
            include_profile: false,
            include_memory_md: true,
        },
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        rendered.contains("### MEMORY.md"),
        "MEMORY.md header must appear when include_memory_md=true, got:\n{rendered}"
    );
    assert!(
        rendered.contains("terse Rust answers"),
        "MEMORY.md body must be injected when include_memory_md=true"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn render_subagent_system_prompt_skips_memory_md_when_disabled() {
    // Narrow specialists with `omit_memory_md = true` (the default)
    // must NOT see MEMORY.md even when it exists on disk.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_memory_off_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("MEMORY.md"),
        "# Long-term memory\nUser prefers terse Rust answers.",
    )
    .unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a narrow specialist.",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        !rendered.contains("### MEMORY.md"),
        "MEMORY.md must NOT appear when include_memory_md=false, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("terse Rust answers"),
        "MEMORY.md body must not leak when include_memory_md=false"
    );

    let _ = std::fs::remove_dir_all(workspace);
}
