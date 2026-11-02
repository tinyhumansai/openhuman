use super::*;

#[cfg(feature = "mcp")]
#[test]
fn declaring_mcp_servers_opts_a_named_belt_into_tool_search() {
    use tinyagents_definition::ToolScope;

    let mut named = ToolScope::Named(vec!["read_file".into()]);
    opt_named_belt_into_discovery(&mut named);
    opt_named_belt_into_discovery(&mut named);
    assert!(matches!(
        named,
        ToolScope::Named(ref names) if names == &["read_file".to_string(), "tool_search".to_string()]
    ));

    let mut wildcard = ToolScope::Wildcard;
    opt_named_belt_into_discovery(&mut wildcard);
    assert!(matches!(wildcard, ToolScope::Wildcard));
}

#[test]
fn agent_ids_must_be_plain_names() {
    assert!(validate_agent_id("ok-agent_1").is_ok());
    assert!(validate_agent_id("../escape").is_err());
}

#[test]
fn config_fn_final_autonomy_is_captured_on_the_built_agent_context() {
    let tokio = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
        .enable_all()
        .build()
        .unwrap();
    let test = tokio.spawn(async {
        use openhuman_core::config::{AutonomyConfig, Config};
        use openhuman_core::core::runtime::AutonomyTierSource;
        use openhuman_core::security::AutonomyLevel;

        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            config_path: dir.path().join("config.toml"),
            workspace_dir: dir.path().join("workspace"),
            action_dir: dir.path().join("action"),
            autonomy: AutonomyConfig {
                enabled: true,
                level: AutonomyLevel::ReadOnly,
                ..AutonomyConfig::default()
            },
            ..Config::default()
        };
        config.save().await.unwrap();

        let runtime = crate::runtime::RuntimeBuilder::new()
            .config(config)
            .services(openhuman_core::core::runtime::ServiceSet::none())
            .build()
            .await
            .expect("runtime with a file-backed ReadOnly config");
        assert_eq!(
            runtime.core_runtime().context().autonomy_tier_source(),
            AutonomyTierSource::Captured,
            "a CoreBuilder::config snapshot remains bound to its in-memory tier"
        );
        let agent = runtime
            .agent(crate::AgentSpec::new("config-fn-tier").config(|effective| {
                effective.autonomy.enabled = true;
                effective.autonomy.level = AutonomyLevel::Full;
            }))
            .expect("AgentSpec config_fn builds");

        assert_eq!(agent.inner.config.autonomy.level, AutonomyLevel::Full);
        assert_eq!(
            agent.inner.ctx.autonomy_tier_source(),
            AutonomyTierSource::Captured
        );
        assert_eq!(
            agent.inner.ctx.agent_policy().unwrap().autonomy,
            AutonomyLevel::Full,
            "the final config_fn tier is what the captured policy carries"
        );

        drop(agent);
        drop(runtime);
    });
    tokio.block_on(test).unwrap();
}

#[test]
fn an_agent_without_sub_agents_resolves_through_the_process_catalogue() {
    let catalogue = process_catalogue();
    let mut definition = super::super::AgentDefinitionSpec::new()
        .into_core("solo")
        .unwrap();
    assert!(
        own_catalogue(&catalogue, "solo", &mut definition, Vec::new())
            .unwrap()
            .is_none()
    );
}

#[test]
fn sub_agents_become_workers_in_a_catalogue_of_the_agents_own() {
    use tinyagents_definition::AgentTier;

    let catalogue = process_catalogue();
    let mut definition = super::super::AgentDefinitionSpec::new()
        .into_core("lead")
        .unwrap();
    let own = own_catalogue(
        &catalogue,
        "lead",
        &mut definition,
        vec![(
            "lead-helper".to_string(),
            super::super::AgentDefinitionSpec::new().when_to_use("helps the lead"),
        )],
    )
    .unwrap()
    .expect("an own catalogue");

    let helper = own.get("lead-helper").expect("the helper is listed");
    assert_eq!(helper.agent_tier, AgentTier::Worker);
    assert!(helper.subagents.is_empty());
    assert!(!helper.extensions.searches_connected_mcp);
    assert_eq!(helper.when_to_use, "helps the lead");
    assert!(own.get("lead").is_some(), "the agent itself is listed");
    assert!(
        own.get("orchestrator").is_some(),
        "built-ins stay reachable"
    );
    assert!(definition
        .subagents
        .iter()
        .any(|entry| matches!(entry, SubagentEntry::AgentId(id) if id == "lead-helper")));
    assert!(catalogue.get("lead-helper").is_none());
}

#[test]
fn sub_agent_ids_are_checked_like_agent_ids() {
    let catalogue = process_catalogue();
    let spec = super::super::AgentDefinitionSpec::new;
    let mut definition = spec().into_core("lead").unwrap();

    let reserved = own_catalogue(
        &catalogue,
        "lead",
        &mut definition,
        vec![("vision_agent".to_string(), spec())],
    );
    assert!(matches!(reserved, Err(AgentError::ReservedId(id)) if id == "vision_agent"));

    let invalid = own_catalogue(
        &catalogue,
        "lead",
        &mut definition,
        vec![("Not Valid".to_string(), spec())],
    );
    assert!(matches!(invalid, Err(AgentError::InvalidId { .. })));

    let duplicate = own_catalogue(
        &catalogue,
        "lead",
        &mut definition,
        vec![("twin".to_string(), spec()), ("twin".to_string(), spec())],
    );
    assert!(matches!(duplicate, Err(AgentError::DuplicateId(id)) if id == "twin"));

    let itself = own_catalogue(
        &catalogue,
        "lead",
        &mut definition,
        vec![("lead".to_string(), spec())],
    );
    assert!(matches!(itself, Err(AgentError::DuplicateId(_))));
}
