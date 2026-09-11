use super::*;

#[test]
fn agent_builder_default_matches_new() {
    let builder = AgentBuilder::new();
    let default_builder = AgentBuilder::default();

    assert_eq!(builder.learning_enabled, default_builder.learning_enabled);
    assert_eq!(builder.auto_save, default_builder.auto_save);
    assert!(builder.turn_model_source.is_none());
    assert!(builder.tools.is_none());
    assert!(builder.memory.is_none());
    assert!(builder.event_session_id.is_none());
    assert!(builder.event_channel.is_none());
    assert!(builder.agent_definition_name.is_none());
    assert!(builder.post_turn_hooks.is_empty());
}
