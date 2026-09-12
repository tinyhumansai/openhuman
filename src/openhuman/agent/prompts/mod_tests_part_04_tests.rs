use super::*;

#[test]
fn agents_md_section_truncates_oversized_layer_at_cap() {
    // One char over the cap forces truncation with a marker.
    let huge = "x".repeat(BOOTSTRAP_MAX_CHARS + 500);
    let ctx = agents_md_ctx(Some(huge), None);
    let out = AgentsInstructionsSection.build(&ctx).unwrap();
    assert!(
        out.contains("truncated"),
        "expected a truncation marker, got tail: {}",
        &out[out.len().saturating_sub(120)..]
    );
    // The rendered block must not carry the full oversized body.
    assert!(
        out.matches('x').count() <= BOOTSTRAP_MAX_CHARS,
        "content must be capped at BOOTSTRAP_MAX_CHARS"
    );
}

#[test]
fn agents_md_section_registered_in_default_builder() {
    let ctx = agents_md_ctx(Some("DEFAULT_BUILDER_MARKER".into()), None);
    let rendered = SystemPromptBuilder::with_defaults().build(&ctx).unwrap();
    assert!(
        rendered.contains("## Project instructions (AGENTS.md)"),
        "with_defaults() must include the AGENTS.md section"
    );
    assert!(rendered.contains("DEFAULT_BUILDER_MARKER"));
    // Ordering contract: AGENTS.md after user-context, before the tool catalogue.
    let agents_pos = rendered
        .find("## Project instructions (AGENTS.md)")
        .unwrap();
    let tools_pos = rendered.find("## Tools").unwrap();
    assert!(
        agents_pos < tools_pos,
        "AGENTS.md must render before the ## Tools catalogue"
    );
}

#[test]
fn agents_md_section_registered_in_dynamic_builder() {
    // The primary/orchestrator + welcome + integrations_agent path:
    // `PromptSource::Dynamic` agents assemble their own body via `render_*`
    // helpers and never call `render_agents_md` individually, so the shared
    // AGENTS.md section is injected centrally in `from_dynamic`. Without this
    // the main chat agent would load AGENTS.md but silently drop it from the
    // system prompt.
    fn dynamic_body(_ctx: &PromptContext<'_>) -> anyhow::Result<String> {
        Ok("DYNAMIC_AGENT_BODY".to_string())
    }
    let ctx = agents_md_ctx(Some("DYNAMIC_GLOBAL_MARKER".into()), None);
    let rendered = SystemPromptBuilder::from_dynamic(dynamic_body)
        .build(&ctx)
        .unwrap();
    assert!(
        rendered.contains("DYNAMIC_AGENT_BODY"),
        "the dynamic agent body must render"
    );
    assert!(
        rendered.contains("## Project instructions (AGENTS.md)"),
        "from_dynamic() must include the AGENTS.md section for the main/orchestrator agent"
    );
    assert!(rendered.contains("DYNAMIC_GLOBAL_MARKER"));
    // Ordering contract: the agent's own body renders first, AGENTS.md follows
    // as trailing standing guidance (before the central grounding suffix).
    let body_pos = rendered.find("DYNAMIC_AGENT_BODY").unwrap();
    let agents_pos = rendered
        .find("## Project instructions (AGENTS.md)")
        .unwrap();
    assert!(
        body_pos < agents_pos,
        "AGENTS.md must render after the dynamic agent body"
    );
}

#[test]
fn agents_md_section_registered_in_subagent_builder() {
    let ctx = agents_md_ctx(None, Some("SUBAGENT_BUILDER_MARKER".into()));
    let builder = SystemPromptBuilder::for_subagent("role body".into(), true, true, false);
    let rendered = builder.build(&ctx).unwrap();
    assert!(
        rendered.contains("## Project instructions (AGENTS.md)"),
        "for_subagent() must include the AGENTS.md section"
    );
    assert!(rendered.contains("SUBAGENT_BUILDER_MARKER"));
}

#[test]
fn agents_md_section_absent_from_prompt_when_gate_off_yields_none() {
    // The config gate produces `None`/`None` (loader not called); the section
    // must then contribute nothing to either builder — no heading leak.
    let ctx = agents_md_ctx(None, None);
    let rendered = SystemPromptBuilder::with_defaults().build(&ctx).unwrap();
    assert!(
        !rendered.contains("## Project instructions (AGENTS.md)"),
        "gated-off (None/None) must not emit the AGENTS.md heading"
    );
}

#[test]
fn subagent_renderer_injects_agents_md_before_tools() {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt_with_format(
        Path::new("/tmp"),
        "reasoning-v1",
        &[0],
        &tools,
        &[],
        "You are a specialist.",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::PFormat,
        &[],
        Some("WS_AGENTS_MARKER"),
        Some("PROJ_AGENTS_MARKER"),
    );
    assert!(rendered.contains("## Project instructions (AGENTS.md)"));
    assert!(rendered.contains("WS_AGENTS_MARKER"));
    assert!(rendered.contains("PROJ_AGENTS_MARKER"));
    let agents_pos = rendered
        .find("## Project instructions (AGENTS.md)")
        .expect("agents heading present");
    let tools_pos = rendered.find("## Tools").expect("tools heading present");
    assert!(
        agents_pos < tools_pos,
        "AGENTS.md must render before the tool catalogue in the subagent renderer"
    );
}

#[test]
fn subagent_renderer_omits_agents_md_when_none() {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        Path::new("/tmp"),
        "reasoning-v1",
        &[0],
        &tools,
        &[],
        "You are a specialist.",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::PFormat,
        &[],
    );
    assert!(
        !rendered.contains("## Project instructions (AGENTS.md)"),
        "public wrapper passes None/None and must emit no AGENTS.md block"
    );
}

#[test]
fn skills_catalog_is_rendered_for_dynamic_and_subagent_builders() {
    let workflows = vec![crate::openhuman::skills::Workflow {
        name: "Release helper".into(),
        dir_name: "release-helper".into(),
        description: "Publishes a release".into(),
        ..Default::default()
    }];
    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &[],
        workflows: &workflows,
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

    fn dynamic_body(_ctx: &PromptContext<'_>) -> anyhow::Result<String> {
        Ok("dynamic body".into())
    }
    let dynamic = SystemPromptBuilder::from_dynamic_with_skills_catalog(dynamic_body, true)
        .build(&ctx)
        .unwrap();
    assert!(dynamic.contains("## Available Skills"));
    assert!(dynamic.contains("release-helper"));

    let subagent = SystemPromptBuilder::for_subagent("inline body".into(), true, true, true)
        .build(&ctx)
        .unwrap();
    assert!(subagent.contains("## Available Skills"));
    assert!(subagent.contains("Publishes a release"));
}

#[test]
fn workflow_catalog_targets_generated_workspace_heading() {
    let workflows = vec![crate::openhuman::skills::Workflow {
        name: "Catalog skill".into(),
        description: "Catalog description".into(),
        ..Default::default()
    }];
    let rendered = render_subagent_system_prompt_with_format_and_workflows(
        Path::new("/tmp"),
        "test-model",
        &[],
        &[],
        &[],
        "An example:\n```markdown\n## Workspace\n```",
        SubagentRenderOptions {
            include_skills_catalog: true,
            ..SubagentRenderOptions::narrow()
        },
        ToolCallFormat::PFormat,
        &[],
        &workflows,
        None,
        None,
    );
    let catalog = rendered.find("## Available Skills").unwrap();
    let workspace = rendered.rfind("## Workspace").unwrap();
    assert!(catalog > rendered.find("```markdown").unwrap());
    assert!(catalog < workspace);
    assert!(rendered.contains("Catalog description"));
}
