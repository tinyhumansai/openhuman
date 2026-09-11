use super::*;
/// The three spec views an agent keeps share their leaf schemas.
///
/// `durable_tool_specs` is a prefix of `tool_specs`, and `visible_tool_specs`
/// is a filtered subset of it. Before openhuman#6218 each was an independent
/// `Vec<ToolSpec>`, so every JSON-Schema `parameters` value was resident up to
/// three times per live agent — ~1.1 MiB of the ~2.5 MiB marginal cost of a
/// `fleet` agent. Pointer identity is the property that keeps it at one copy,
/// so assert it directly rather than asserting equal contents (which the old
/// deep-cloning shape also satisfied).
///
/// One spec is exempt by design — see the `use_skill` branch below.
#[test]
fn the_three_spec_views_share_their_leaf_schemas() {
    crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global_builtins().unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    let agent = crate::openhuman::agent::Agent::from_config_for_agent(&config, "orchestrator")
        .expect("orchestrator session build");

    let durable = agent.durable_tool_specs_arc();
    let all = agent.tool_specs_arc();
    let visible = agent.visible_tool_specs_arc();

    assert!(
        !durable.is_empty(),
        "the orchestrator carries a durable registry, so there is something to share"
    );
    assert!(
        durable.len() <= all.len(),
        "the durable set is a prefix of the full set"
    );

    for (i, spec) in durable.iter().enumerate() {
        assert!(
            std::sync::Arc::ptr_eq(spec, &all[i]),
            "durable spec `{}` must be the same allocation as the full view's entry, \
             not a deep copy",
            spec.name
        );
    }

    assert!(
        !visible.is_empty(),
        "the orchestrator advertises tools, so the visible view is non-empty"
    );
    // `use_skill` is the one deliberate exception, and it is deliberate in the
    // other direction: `visible_tool_specs_for_policy` rewrites its pack index
    // and `skill` enum down to the packs THIS session can actually call, so the
    // visible entry must NOT be the durable one — `durable_tool_specs` stays the
    // unscoped truth, and sharing the leaf would scope it for every view that
    // holds it. Every other spec is filtered wholesale, never rewritten, so it
    // shares like everything else.
    //
    // Asserted rather than merely excluded: a future change that made the
    // rewrite mutate in place would silently scope the durable set, and a
    // change that deep-copied everything again would still pass an
    // exclusion-only test.
    let use_skill = crate::openhuman::tools::toolpacks::USE_SKILL;
    let mut saw_scoped_use_skill = false;

    for spec in visible.iter() {
        let shared = all
            .iter()
            .any(|candidate| std::sync::Arc::ptr_eq(candidate, spec));
        if spec.name == use_skill {
            assert!(
                !shared,
                "`{use_skill}` must be scoped into its own allocation — sharing the leaf would \
                 rewrite the durable set's copy too"
            );
            saw_scoped_use_skill = true;
            continue;
        }
        assert!(
            shared,
            "visible spec `{}` must point at the full view's allocation, not a deep copy",
            spec.name
        );
    }

    assert!(
        saw_scoped_use_skill,
        "the orchestrator advertises `{use_skill}`, so the scoped-copy exception above \
         must actually have been exercised rather than vacuously skipped"
    );
}

/// Failure path: a duplicate name must not smuggle a *different* allocation
/// through the dedup.
///
/// `dedup_visible_tool_specs` keeps the first occurrence. With shared leaves
/// the survivor must still be the exact entry that was handed in — a helper
/// that rebuilt the kept spec would reintroduce the per-agent copy the sharing
/// exists to remove, while every content-equality assertion still passed.
#[test]
fn dedup_keeps_the_original_allocation_of_the_winning_spec() {
    let first = std::sync::Arc::new(spec("research"));
    let mut shadow = spec("research");
    shadow.description = "the delegate that must lose".to_string();
    let shadow = std::sync::Arc::new(shadow);
    let other = std::sync::Arc::new(spec("plan"));

    let deduped = dedup_visible_tool_specs(vec![
        std::sync::Arc::clone(&first),
        std::sync::Arc::clone(&other),
        std::sync::Arc::clone(&shadow),
    ]);

    assert_eq!(deduped.len(), 2, "the shadowing duplicate must be dropped");
    assert!(
        std::sync::Arc::ptr_eq(&deduped[0], &first),
        "the surviving `research` spec must be the first allocation, not a rebuild"
    );
    assert!(
        !std::sync::Arc::ptr_eq(&deduped[0], &shadow),
        "the shadowing delegate's schema must not reach the provider"
    );
    assert!(std::sync::Arc::ptr_eq(&deduped[1], &other));
}

/// Failure path: an agent with no tools at all must still build, and its three
/// spec views must be empty rather than desynchronised.
#[test]
fn spec_views_stay_consistent_for_a_tool_less_agent() {
    let model: std::sync::Arc<dyn tinyinference::model::ChatModel<()>> =
        std::sync::Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    let agent = crate::openhuman::agent::AgentBuilder::new()
        .chat_model(model)
        .tools(Vec::new())
        .memory(crate::openhuman::memory::test_support::noop_memory())
        .tool_dispatcher(Box::new(
            crate::openhuman::agent::dispatcher::XmlToolDispatcher,
        ))
        .build()
        .expect("a tool-less agent is a legal build");

    assert!(agent.tool_specs().is_empty());
    assert!(agent.durable_tool_specs_arc().is_empty());
    assert!(agent.visible_tool_specs_arc().is_empty());
}
