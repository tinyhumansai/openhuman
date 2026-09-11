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
    crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("builtin agent definitions must load");
    let registry = crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
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
    crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("builtin agent definitions must load");

    // A visible set shaped like the live one: the advertised delegates are in,
    // the packed ones are not.
    let visible: HashSet<String> = ["research", "plan", "ask_docs", "file_read", "goal_complete"]
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
    assert!(
        block.contains("skill `documents`, tool `make_presentation`"),
        "a packed delegate must render with its route:\n{block}"
    );
}

/// The row text must be one readable sentence, not a cut parenthetical.
///
/// `mcp_agent`'s `when_to_use` opens "…an ALREADY-CONNECTED MCP server (e.g.
/// `gmail`)…", and a naive split on ". " ends the row at "(e.g." — which is
/// what the first live capture rendered.
#[test]
fn a_row_is_not_cut_at_an_abbreviation() {
    assert_eq!(
        first_sentence("Calls tools on a connected server (e.g. gmail). Then reports back."),
        "Calls tools on a connected server (e.g. gmail).",
    );
    // A genuine boundary still ends the row.
    assert_eq!(
        first_sentence("Builds decks from evidence. Use for pitch-deck requests."),
        "Builds decks from evidence.",
    );
    // No boundary at all: capped, not truncated mid-word by accident.
    let long = "a ".repeat(200);
    assert!(first_sentence(&long).ends_with('…'));
    // Short and unterminated: returned whole.
    assert_eq!(
        first_sentence("Runs installed agent skills"),
        "Runs installed agent skills"
    );
}

/// The generated intro must not carry the source's line-continuation padding.
#[test]
fn the_generated_block_has_no_stray_whitespace_runs() {
    crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("builtin agent definitions must load");
    let visible: HashSet<String> = ["research".to_string()].into_iter().collect();
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
        ARCHETYPE.contains("Workflow rule of thumb"),
        "orchestrator prompt must carry the workflow routing rule"
    );
    assert!(
        ARCHETYPE.contains("`build_workflow`"),
        "the rule must name the delegate to call"
    );
    assert!(
        ARCHETYPE.contains("use_skill"),
        "the rule must name the path it is steering away from"
    );

    // The rule is only true because these are the real names. Asserting the
    // prompt against itself would survive a rename of either side; asserting it
    // against the pack and the agent definition does not.
    let pack =
        crate::openhuman::tools::toolpacks::pack("workflows").expect("the workflows pack exists");
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

    let registry =
        crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::builtins_only();
    let builder = registry
        .get("workflow_builder")
        .expect("workflow_builder is a registered agent");
    assert_eq!(
        builder.delegate_name.as_deref(),
        Some("build_workflow"),
        "the prompt tells the model to call `build_workflow`; that must still be \
         workflow_builder's delegate_name, or the rule names a tool nobody has"
    );
}
