use super::*;
use serde_json::json;
use tinychannels_bus::controllers::{
    BotPermissionCheck, DiscordChannelListResult, DiscordGuild, DiscordGuildListResult,
};

#[tokio::test]
async fn discord_credentials_never_reach_an_unattested_bus() {
    use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};
    #[derive(Clone)]
    struct Unattested(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    #[tinybus::interface(name = "test.ChannelController")]
    impl Unattested {
        async fn channel_controller(
            &self,
            _: ChannelControllerRequest,
        ) -> tinybus::Result<ChannelControllerResult> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(ChannelControllerResult::DiscordGuilds(Default::default()))
        }
    }
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    service
        .serve_at(
            ObjectPath::new("/test/ChannelController").unwrap(),
            Unattested(calls.clone()),
        )
        .await
        .unwrap();
    service
        .request_name("test.ChannelController")
        .await
        .unwrap();
    let proxy = caller
        .proxy(
            "test.ChannelController",
            "/test/ChannelController",
            "test.ChannelController",
        )
        .unwrap();
    let result = discord::<Vec<DiscordGuild>>(
        &crate::modules::ops::fixture_client(proxy.clone()),
        ChannelControllerRequest::DiscordListGuilds {
            config: Default::default(),
            bot_token: "fixture-token".into(),
            proxy: Default::default(),
        },
        DiscordReply::Guilds,
    )
    .await;
    assert_eq!(result.unwrap_err(), ModuleCallError::TransportFailed);
    let prepared = prepare_connection(
        &crate::modules::ops::fixture_client(proxy),
        ChannelControllerRequest::PrepareConnect {
            config: Default::default(),
            channel: "discord".into(),
            auth_mode: tinychannels_bus::ChannelAuthMode::BotToken,
            credentials: json!({"bot_token":"fixture-token"}),
            proxy: Default::default(),
        },
    )
    .await;
    assert!(matches!(prepared, Err(ModuleCallError::TransportFailed)));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn discord_guild_projection_preserves_icons_and_module_order() {
    let wire = json!([
        {"id":"second", "name":"Second", "icon":null},
        {"id":"first", "name":"First", "icon":"icon-hash"}
    ]);
    let guilds: Vec<DiscordGuild> = decode_discord(
        ChannelControllerResult::DiscordGuilds(DiscordGuildListResult {
            raw: Some(wire.clone()),
            ..Default::default()
        }),
        DiscordReply::Guilds,
    )
    .unwrap();
    assert_eq!(serde_json::to_value(guilds).unwrap(), wire);
}

#[test]
fn discord_permission_projection_keeps_all_legacy_permission_facts() {
    let wire = json!({"can_view_channel":false,"can_send_messages":false,"can_read_message_history":true,"missing_permissions":["VIEW_CHANNEL", "SEND_MESSAGES"]});
    let result: BotPermissionCheck = decode_discord(
        ChannelControllerResult::DiscordPermissions(
            tinychannels_bus::controllers::DiscordPermissionCheckResult {
                raw: Some(wire.clone()),
                ..Default::default()
            },
        ),
        DiscordReply::Permissions,
    )
    .unwrap();
    assert_eq!(serde_json::to_value(result).unwrap(), wire);
}

#[test]
fn discord_projection_rejects_wrong_result_kind_or_missing_native_fields() {
    assert_eq!(
        decode_discord::<Vec<DiscordGuild>>(
            ChannelControllerResult::DiscordChannels(DiscordChannelListResult {
                raw: Some(json!([])),
                ..Default::default()
            }),
            DiscordReply::Guilds
        )
        .unwrap_err(),
        ModuleCallError::ModuleFault
    );
    assert_eq!(
        decode_discord::<BotPermissionCheck>(
            ChannelControllerResult::DiscordPermissions(
                tinychannels_bus::controllers::DiscordPermissionCheckResult {
                    raw: Some(json!({"can_send_messages":true})),
                    ..Default::default()
                }
            ),
            DiscordReply::Permissions
        )
        .unwrap_err(),
        ModuleCallError::ModuleFault
    );
}

#[test]
fn discord_projection_refuses_missing_raw_reply_without_inventing_defaults() {
    assert_eq!(
        decode_discord::<Vec<DiscordGuild>>(
            ChannelControllerResult::DiscordGuilds(DiscordGuildListResult::default()),
            DiscordReply::Guilds
        )
        .unwrap_err(),
        ModuleCallError::ModuleFault
    );
}

#[test]
fn discord_proxy_resolves_only_the_host_selected_service() {
    let mut config = crate::config::Config::default();
    config.proxy.enabled = true;
    config.proxy.scope = crate::config::ProxyScope::Services;
    config.proxy.https_proxy = Some("http://proxy.example.test:3128".into());
    config.proxy.services = vec!["channel.telegram".into()];
    assert!(discord_proxy(&config) == HttpProxyPolicy::default());
    config.proxy.services = vec!["channel.*".into()];
    let policy = discord_proxy(&config);
    assert_eq!(policy.service_selectors, ["channel.discord"]);
    assert_eq!(policy.https_proxy, config.proxy.https_proxy);
}

#[test]
fn discord_channel_projection_preserves_type_position_and_category() {
    let wire =
        json!([{"id":"c", "name":"general", "type":0, "position":7, "parent_id":"category"}]);
    let channels: Vec<tinychannels_bus::controllers::DiscordTextChannel> = decode_discord(
        ChannelControllerResult::DiscordChannels(DiscordChannelListResult {
            raw: Some(wire.clone()),
            ..Default::default()
        }),
        DiscordReply::Channels,
    )
    .unwrap();
    assert_eq!(serde_json::to_value(channels).unwrap(), wire);
}

#[tokio::test]
async fn hosted_identity_requires_the_module_and_never_uses_a_local_key_fallback() {
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;
    let result = prepare_identity(
        &crate::modules::client(config),
        tinychannels_bus::module_inbound::PrepareChannelIdentityRequest {
            channel: "discord:fixture".into(),
            sender: Some("fixture-private-sender".into()),
            reply_target: Some("fixture-private-chat".into()),
            thread_ts: None,
        },
    )
    .await;
    let error = result
        .err()
        .expect("module-disabled identity preparation must fail");
    assert_eq!(error, ModuleCallError::Unavailable);
    assert!(!error.to_string().contains("fixture-private"));
}
