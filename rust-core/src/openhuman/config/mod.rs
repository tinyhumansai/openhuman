pub mod daemon;
pub mod schema;

#[allow(unused_imports)]
pub use daemon::DaemonConfig;

#[allow(unused_imports)]
pub use schema::{
    apply_runtime_proxy_to_builder, build_runtime_proxy_client,
    build_runtime_proxy_client_with_timeouts, runtime_proxy_config, set_runtime_proxy_config,
    ScreenIntelligenceConfig, AgentConfig, AuditConfig, AutonomyConfig,
    BrowserComputerUseConfig, BrowserConfig, ChannelsConfig, ClassificationRule,
    CloudflareTunnelConfig, ComposioConfig, Config, CostConfig, CronConfig, CustomTunnelConfig,
    DelegateAgentConfig, DiscordConfig, DockerRuntimeConfig, EmbeddingRouteConfig, GatewayConfig,
    HardwareConfig, HardwareTransport, HeartbeatConfig, HttpRequestConfig, IMessageConfig,
    IdentityConfig, LarkConfig, LocalAiConfig, MatrixConfig, MemoryConfig, ModelRouteConfig,
    MultimodalConfig, NgrokTunnelConfig, ObservabilityConfig, PeripheralBoardConfig,
    PeripheralsConfig, ProxyConfig, ProxyScope, QueryClassificationConfig, ReliabilityConfig,
    ResourceLimitsConfig, RuntimeConfig, SandboxBackend, SandboxConfig, SchedulerConfig,
    SecretsConfig, SecurityConfig, SlackConfig, StorageConfig, StorageProviderConfig,
    StorageProviderSection, StreamMode, TailscaleTunnelConfig, TelegramConfig, TunnelConfig,
    WebSearchConfig, WebhookConfig,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexported_config_default_is_constructible() {
        let config = Config::default();

        assert!(config.default_provider.is_some());
        assert!(config.default_model.is_some());
        assert!(config.default_temperature > 0.0);
    }

    #[test]
    fn reexported_channel_configs_are_constructible() {
        let telegram = TelegramConfig {
            bot_token: "token".into(),
            allowed_users: vec!["alice".into()],
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1000,
            mention_only: false,
        };

        let discord = DiscordConfig {
            bot_token: "token".into(),
            guild_id: Some("123".into()),
            allowed_users: vec![],
            listen_to_bots: false,
            mention_only: false,
        };

        let lark = LarkConfig {
            app_id: "app-id".into(),
            app_secret: "app-secret".into(),
            encrypt_key: None,
            verification_token: None,
            allowed_users: vec![],
            use_feishu: false,
            receive_mode: crate::openhuman::config::schema::LarkReceiveMode::Websocket,
            port: None,
        };

        assert_eq!(telegram.allowed_users.len(), 1);
        assert_eq!(discord.guild_id.as_deref(), Some("123"));
        assert_eq!(lark.app_id, "app-id");
    }
}
