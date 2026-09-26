use super::*;

#[test]
fn catalog_keeps_the_wire_names() {
    let names: Vec<String> = all_channel_link_controller_schemas()
        .iter()
        .map(|s| format!("{}.{}", s.namespace, s.function))
        .collect();
    assert_eq!(names[0], "auth.create_channel_link_token");
    assert_eq!(
        names[1..],
        [
            "channels.telegram_login_start",
            "channels.telegram_login_check",
            "channels.discord_link_start",
            "channels.discord_link_check",
        ]
    );
    assert_eq!(channel_link_schemas("nope").function, "unknown");
}

#[test]
fn link_token_requires_channel() {
    let s = channel_link_schemas("auth_create_channel_link_token");
    assert!(s.inputs.iter().any(|f| f.name == "channel" && f.required));
    assert!(deserialize_params::<AuthCreateChannelLinkTokenParams>(Map::new()).is_err());
}
