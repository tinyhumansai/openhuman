use super::*;

#[test]
fn custom_delegate_is_treated_as_spawn_tool() {
    assert!(is_subagent_spawn_tool("spawn_subagent"));
    assert!(is_subagent_spawn_tool("spawn_async_subagent"));
    assert!(is_subagent_spawn_tool("delegate_researcher"));
    assert!(!is_subagent_spawn_tool("directory_resolve"));
}

#[test]
fn unprefixed_delegate_name_overrides_are_treated_as_spawn_tools() {
    // Most synthesised delegation tools use an unprefixed
    // `delegate_name` override (`plan`, `manage_tasks`, `create_image`, …).
    // They must be stripped from every sub-agent surface, exactly like
    // the `delegate_*`-prefixed defaults.
    let tmp = tempfile::TempDir::new().unwrap();
    crate::agent::harness::definition::AgentDefinitionRegistry::init_global(tmp.path()).unwrap();
    for delegate in [
        "plan",
        "review_code",
        "manage_tasks",
        "create_image",
        // `make_presentation` is `presentation_agent`'s `delegate_name`; the agent —
        // and therefore this delegate tool — is compiled out with the
        // `documents` feature.
        #[cfg(feature = "documents")]
        "make_presentation",
    ] {
        assert!(
            is_subagent_spawn_tool(delegate),
            "`{delegate}` is a synthesised delegation tool and must be \
             stripped from sub-agent tool surfaces"
        );
    }
    // Ordinary worker tools stay visible.
    for plain in ["shell", "file_read", "web_fetch", "todo"] {
        assert!(
            !is_subagent_spawn_tool(plain),
            "`{plain}` must not be classified as a spawn tool"
        );
    }
}

#[test]
fn child_keeps_the_parents_protocol() {
    use crate::agent::prompts::ToolCallFormat;

    for parent in [
        ToolCallFormat::Native,
        ToolCallFormat::PFormat,
        ToolCallFormat::Json,
    ] {
        let (format, _) = subagent_prompt_protocol(parent, &[]);
        assert_eq!(format, parent);
    }
}

/// A custom agent resolved for spawning keeps its `tool_allowlist` as its
/// scope, so one that lists `spawn_async_subagent` would inherit the parent's
/// instance. The runner's child-surface strip must remove it (#6934).
#[test]
fn custom_agent_allowlisting_spawn_async_subagent_loses_it_on_the_child_surface() {
    use crate::agent::registry::types::{AgentRegistryEntry, AgentRegistrySource};

    let mut config = crate::config::Config::default();
    config.agent_registry.entries = vec![AgentRegistryEntry {
        id: "fan_out_helper".to_string(),
        name: "Fan-out helper".to_string(),
        description: "Custom agent that names the async spawn tool.".to_string(),
        source: AgentRegistrySource::Custom,
        enabled: true,
        model: None,
        system_prompt: Some("Do the work.".to_string()),
        tool_allowlist: vec![
            "spawn_async_subagent".to_string(),
            "current_time".to_string(),
        ],
        tool_denylist: Vec::new(),
        subagents: Default::default(),
        tags: Vec::new(),
        metadata: serde_json::Value::Null,
    }];
    let registry = crate::agent::harness::definition::AgentDefinitionRegistry::builtins_only();
    let definition = crate::agent::registry::resolve_spawnable_definition(
        &registry,
        Some(&config),
        "fan_out_helper",
    )
    .expect("an enabled custom agent resolves for spawning");

    let parent_tools: Vec<Box<dyn tinytools::Tool>> = vec![
        Box::new(crate::agent::orchestration::tools::SpawnAsyncSubagentTool::new()),
        Box::new(tinyagents_harness::tools::CurrentTimeTool::new()),
    ];
    let mut allowed = filter_tool_indices(&parent_tools, &definition.tools, &[], None);
    let scoped: Vec<&str> = allowed.iter().map(|&i| parent_tools[i].name()).collect();
    assert!(
        scoped.contains(&"spawn_async_subagent"),
        "fixture: the custom scope admits the spawn tool before the strip: {scoped:?}"
    );

    allowed.retain(|&i| !is_subagent_spawn_tool(parent_tools[i].name()));
    let child: Vec<&str> = allowed.iter().map(|&i| parent_tools[i].name()).collect();
    assert!(
        !child.contains(&"spawn_async_subagent"),
        "a child must not be able to spawn async sub-agents: {child:?}"
    );
    assert!(
        child.contains(&"current_time"),
        "ordinary tools survive: {child:?}"
    );
}
