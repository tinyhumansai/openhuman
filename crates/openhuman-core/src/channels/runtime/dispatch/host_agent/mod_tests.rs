use super::*;

fn bound(channel: &str, agent_id: &str) -> Config {
    let mut config = Config::default();
    config
        .agent
        .channel_agents
        .insert(channel.to_string(), agent_id.to_string());
    config
}

#[test]
fn an_unbound_channel_names_no_agent() {
    assert_eq!(bound_agent_id(&Config::default(), "telegram"), None);
    assert_eq!(bound_agent_id(&bound("discord", "x"), "telegram"), None);
}

#[test]
fn a_bound_channel_names_its_agent() {
    let config = bound("telegram", "teeny-chat");
    assert_eq!(bound_agent_id(&config, "telegram"), Some("teeny-chat"));
}

#[test]
fn a_blank_binding_is_no_binding() {
    assert_eq!(bound_agent_id(&bound("telegram", "  "), "telegram"), None);
}

#[test]
fn the_binding_is_read_from_toml() {
    let config: crate::config::schema::AgentConfig =
        toml::from_str("[channel_agents]\ntelegram = \"teeny-chat\"\n").unwrap();
    assert_eq!(
        config.channel_agents.get("telegram").map(String::as_str),
        Some("teeny-chat")
    );
}
