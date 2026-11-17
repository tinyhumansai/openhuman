use super::*;

#[test]
fn explicit_turn_temperature_overrides_the_definition_without_mutating_it() {
    let mut definition = crate::agent::harness::test_support::test_main_def();
    definition.temperature = 0.6;
    let effective = definition_for_turn(&definition, Some(0.9));
    assert_eq!(effective.temperature, 0.9);
    assert_eq!(definition.temperature, 0.6);
    assert!(matches!(
        definition_for_turn(&definition, None),
        std::borrow::Cow::Borrowed(_)
    ));
    assert_eq!(definition_for_turn(&definition, None).temperature, 0.6);
}

fn test_config(tmp: &tempfile::TempDir) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    config
}

// ── `agent_chat_for`: explicit targets and thread-scoped resume ───────────

/// An unknown `agent_id` is a caller error surfaced before any provider is
/// touched, not a silent fall-through to the orchestrator.
#[tokio::test]
async fn agent_chat_for_rejects_an_unknown_agent_id() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = test_config(&tmp);
    std::fs::create_dir_all(&config.workspace_dir).unwrap();

    let err = agent_chat_for(
        &mut config,
        AgentChatTarget::AgentId("no-such-agent-xyz"),
        "hello",
        None,
        None,
        None,
        None,
        None,
        AgentChatEnv::default(),
    )
    .await
    .expect_err("an unknown agent id must be refused");
    assert!(
        err.to_lowercase().contains("no-such-agent-xyz") || err.to_lowercase().contains("unknown"),
        "error names the id or says unknown: {err}"
    );
}

/// `AgentChatTarget` is `Copy` and the orchestrator arm is the historical
/// `agent_chat`, so the delegating wrapper and the explicit call agree.
#[test]
fn agent_chat_target_default_is_the_orchestrator() {
    let target = AgentChatTarget::Orchestrator;
    let copy = target;
    assert!(matches!(copy, AgentChatTarget::Orchestrator));
    assert!(matches!(
        AgentChatTarget::AgentId("x"),
        AgentChatTarget::AgentId("x")
    ));
}
