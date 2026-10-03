use super::*;

/// The web channel renames the session, and the block must survive it.
///
/// `PromptContext::agent_id` carries `agent_definition_name`, which the web
/// channel rewrites to `orchestrator_<short_thread>`. A plain `registry.get`
/// on that misses, and the whole withheld-specialist block silently vanishes —
/// which is exactly what the first live capture showed: the routing table was
/// gone from the prompt and nothing had replaced it.
#[test]
fn a_thread_renamed_session_still_resolves_to_its_registry_entry() {
    // The process-global registry is not initialised in unit tests, and
    // initialising it here would leak into every other test in the binary.
    crate::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("builtin agent definitions must load");
    let registry = crate::agent::harness::definition::AgentDefinitionRegistry::global()
        .expect("init_global_builtins publishes the registry");

    let exact = resolve_definition(registry, "orchestrator").expect("exact id must resolve");
    assert_eq!(exact.id, "orchestrator");

    let renamed = resolve_definition(registry, "orchestrator_thread-captu")
        .expect("a thread-renamed session must resolve to its registry entry");
    assert_eq!(renamed.id, "orchestrator");

    // Not a rename, just a different agent: must not be swallowed by a
    // shorter id that happens to be a prefix.
    assert!(
        resolve_definition(registry, "orchestratorish").is_none(),
        "a name that merely starts with an id is not that agent"
    );
}

/// The generated block must actually render for a real, renamed session.
///
/// The first live capture had every `prompt.md` edit in it and no block, which
/// only a silent early return can produce. Pin the whole path: a renamed agent
/// id, a visible set that withholds the packed delegates, and at least one row
/// naming a real route.
#[test]
fn the_withheld_block_renders_for_a_renamed_session_with_a_filter() {
    crate::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("builtin agent definitions must load");

    // A visible set shaped like the live one: the advertised delegates are in,
    // the packed ones are not.
    let visible: HashSet<String> = ["file_read", "goal_complete"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut ctx = ctx_with(&[]);
    ctx.agent_id = "orchestrator_thread-captu";
    ctx.visible_tool_names = &visible;

    let block = render_withheld_specialists(&ctx);
    assert!(
        block.starts_with("## Capabilities not in your tool list"),
        "expected the generated heading, got: {:?}",
        block.chars().take(120).collect::<String>()
    );
    // A packed delegate no Cargo feature gates must always render with its
    // route; this is the row that pins the block in every build profile.
    assert!(
        block.contains("`tasks` (manage_tasks"),
        "a packed delegate must render with its route:\n{block}"
    );
    // `make_presentation` is a `documents`-gated skill: under `default` it is
    // not compiled and its row cannot render, so expecting it unconditionally
    // reads as a broken block (which is how openhuman#6507 came to be filed
    // and retracted). Assert it exactly when the feature is on, and assert its
    // absence when it is off, so the expectation tracks the build profile.
    let presentation_row = block.contains("`documents` (make_presentation");
    assert_eq!(
        presentation_row,
        cfg!(feature = "documents"),
        "the `documents` row must render exactly when the `documents` feature is enabled \
         (see openhuman#6512):\n{block}"
    );
}

/// The generated intro must not carry the source's line-continuation padding.
#[test]
fn the_generated_block_has_no_stray_whitespace_runs() {
    crate::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("builtin agent definitions must load");
    let visible: HashSet<String> = ["file_read".to_string()].into_iter().collect();
    let mut ctx = ctx_with(&[]);
    ctx.agent_id = "orchestrator";
    ctx.visible_tool_names = &visible;
    let block = render_withheld_specialists(&ctx);
    assert!(!block.is_empty(), "expected a rendered block");
    assert!(
        !block.contains("  "),
        "the block carries doubled spaces from the source literal:\n{block}"
    );
}

#[test]
fn prompt_routes_workflow_authoring_to_the_builder_not_use_skill() {
    // Regression for the `use_skill` routing dead end. The orchestrator loaded
    // the `workflows` pack, read `propose_workflow` off the listing, called it
    // through `use_skill`, and was refused — six times, until the
    // repeated-failure breaker killed the turn.
    //
    // The gate is the fix; this pins the prompt so the model is told the route
    // before it discovers the wall.
    assert!(
        ARCHETYPE.contains(
            "Build or edit a workflow: spawn `workflow_builder` with `spawn_async_subagent`"
        ),
        "orchestrator prompt must carry the workflow routing rule and name the spawn to make"
    );

    // The rule is only true because these are the real names. Asserting the
    // prompt against itself would survive a rename of either side; asserting it
    // against the pack and the agent definition does not.
    let pack = crate::tools::toolpacks::pack("workflows").expect("the workflows pack exists");
    assert!(
        pack.tools.contains(&"propose_workflow"),
        "the prompt names propose_workflow as pack-owned: {:?}",
        pack.tools
    );
    assert!(
        pack.owners.contains(&"workflow_builder"),
        "the prompt routes to workflow_builder as an owner: {:?}",
        pack.owners
    );

    let registry = crate::agent::harness::definition::AgentDefinitionRegistry::builtins_only();
    let builder = registry
        .get("workflow_builder")
        .expect("workflow_builder is a registered agent");
    assert_eq!(
        builder.delegate_name.as_deref(),
        Some("build_workflow"),
        "the prompt tells the model to call `build_workflow`; that must still be \
         workflow_builder's delegate_name, or the rule names a tool nobody has"
    );
    match &builder.tools {
        crate::agent::harness::definition::ToolScope::Named(tools) => {
            for tool in ["list_flows", "get_flow"] {
                assert!(
                    tools.contains(&tool.to_string()),
                    "the saved-flow lookup route needs `{tool}` on workflow_builder's belt"
                );
            }
        }
        crate::agent::harness::definition::ToolScope::Wildcard => {
            panic!("workflow_builder must retain its explicit, narrow tool belt")
        }
    }
}

/// #6302: the hand-off the skills sections name is the call this
/// session can make right now: direct when it is on the belt, the `use_skill`
/// form when a pack holds it, and nothing when the agent has no route.
#[cfg(all(feature = "mcp", feature = "skills"))]
#[test]
fn skill_sections_name_the_hand_off_this_session_can_call() {
    crate::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("builtin agent definitions must load");
    let belt: HashSet<String> = ["setup_skills", "run_workflow", "file_read", "use_skill"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut ctx = ctx_with(&[]);
    ctx.agent_id = "orchestrator";
    ctx.visible_tool_names = &belt;

    assert_eq!(
        hand_off_route(&ctx, "skill_setup").as_deref(),
        Some("`setup_skills`")
    );
    // Running a skill is the orchestrator's own `run_workflow`, not a hand-off.
    assert_eq!(run_workflow_route(&ctx).as_deref(), Some("`run_workflow`"));
    // Listed but held by a pack: name the call that actually reaches it.
    assert_eq!(
        hand_off_route(&ctx, "image_agent").as_deref(),
        Some("`create_image` (`use_skill` skill `media`)")
    );
    // Not in the orchestrator's allowlist: no route, so name nothing. `planner`
    // is registered for workflow runs but is not a chat delegate.
    assert_eq!(hand_off_route(&ctx, "summarizer"), None);
    assert_eq!(hand_off_route(&ctx, "planner"), None);

    // The generated withheld block no longer lists the unpacked hand-offs.
    let block = render_withheld_specialists(&ctx);
    for handoff in ["setup_skills", "run_workflow"] {
        assert!(
            !block.contains(handoff),
            "`{handoff}` is a direct tool and must not be listed as withheld:\n{block}"
        );
    }

    // A packed route needs `use_skill` on the belt. A session filtered down to
    // neither the delegate nor `use_skill` cannot reach the specialist at all,
    // and naming a call it cannot make is the bug, not the fix.
    let no_use_skill: HashSet<String> = ["setup_skills", "file_read"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    ctx.visible_tool_names = &no_use_skill;
    assert_eq!(
        hand_off_route(&ctx, "image_agent"),
        None,
        "without `use_skill` there is no packed route to name"
    );
    assert_eq!(
        run_workflow_route(&ctx),
        None,
        "without `run_workflow` on the belt there is no way to run a skill"
    );
    assert_eq!(
        hand_off_route(&ctx, "skill_setup").as_deref(),
        Some("`setup_skills`"),
        "a delegate on the belt is still a direct route"
    );
}

/// A row `prompt.md` tags for a family this build lacks must not reach the
/// model, and the tag itself never does.
#[test]
fn a_route_tagged_prompt_row_is_dropped_when_its_family_is_absent() {
    let md = "keep me\n   - Skills row<!--route:skills-->\n   - MCP row<!--route:mcp-->\ntail";

    let both = strip_route_lines(md, true, true, true);
    assert!(
        both.contains("Skills row") && both.contains("MCP row"),
        "both rows survive when both families are present: {both}"
    );
    assert!(
        !both.contains("<!--route:"),
        "the tag is an authoring marker and must never reach the model: {both}"
    );

    let neither = strip_route_lines(md, false, false, true);
    assert!(
        !neither.contains("Skills row") && !neither.contains("MCP row"),
        "a row whose family is compiled out must be dropped: {neither}"
    );
    assert!(
        neither.contains("keep me") && neither.contains("tail"),
        "untagged prose is untouched: {neither}"
    );

    let skills_only = strip_route_lines(md, true, false, true);
    assert!(
        skills_only.contains("Skills row") && !skills_only.contains("MCP row"),
        "each tag is decided on its own: {skills_only}"
    );
}

#[test]
fn the_composio_connect_row_is_dropped_when_composio_is_off() {
    let md = "keep\n- connect: `composio_connect`<!--route:composio-->\ntail";
    let on = strip_route_lines(md, true, true, true);
    assert!(on.contains("composio_connect") && !on.contains("<!--route:"));
    let off = strip_route_lines(md, true, true, false);
    assert!(!off.contains("composio_connect"), "{off}");
    assert!(off.contains("keep") && off.contains("tail"));
}
