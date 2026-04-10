//! Configuration management for the OpenHuman core.
//!
//! This module serves as the primary gateway for all configuration-related functionality.
//! It re-exports types and functions from submodules to provide a unified API for:
//! - Loading and saving user settings (`Config`).
//! - Managing the core daemon's lifecycle and options (`DaemonConfig`).
//! - Defining the RPC surface for configuration management.
//! - Handling the schema definitions for all agent and system settings.

pub mod daemon;
pub mod ops;
pub mod schema;
mod schemas;
pub mod settings_cli;

#[allow(unused_imports)]
pub use daemon::DaemonConfig;

/// RPC operations for configuration.
pub use ops as rpc;
pub use ops::*;

#[allow(unused_imports)]
pub use schema::{
    apply_runtime_proxy_to_builder, build_runtime_proxy_client,
    build_runtime_proxy_client_with_timeouts, runtime_proxy_config, set_runtime_proxy_config,
    AgentConfig, ArchetypeConfig, AuditConfig, AutocompleteConfig, AutonomyConfig,
    BrowserComputerUseConfig, BrowserConfig, ChannelsConfig, ClassificationRule, ComposioConfig,
    Config, CostConfig, CronConfig, DelegateAgentConfig, DictationActivationMode, DictationConfig,
    DiscordConfig, DockerRuntimeConfig, EmbeddingRouteConfig, HardwareConfig, HardwareTransport,
    HeartbeatConfig, HttpRequestConfig, IMessageConfig, IdentityConfig, IntegrationToggle,
    IntegrationsConfig, LarkConfig, LearningConfig, LocalAiConfig, MatrixConfig, MemoryConfig,
    ModelRouteConfig, MultimodalConfig, ObservabilityConfig, OrchestratorConfig,
    PeripheralBoardConfig, PeripheralsConfig, ProxyConfig, ProxyScope, QueryClassificationConfig,
    ReflectionSource, ReliabilityConfig, ResourceLimitsConfig, RuntimeConfig, SandboxBackend,
    SandboxConfig, SchedulerConfig, ScreenIntelligenceConfig, SecretsConfig, SecurityConfig,
    SlackConfig, StorageConfig, StorageProviderConfig, StorageProviderSection, StreamMode,
    TelegramConfig, UpdateConfig, VoiceActivationMode, VoiceServerConfig, WebSearchConfig,
    WebhookConfig, DEFAULT_MODEL, MODEL_AGENTIC_V1, MODEL_CODING_V1, MODEL_REASONING_V1,
};
pub use schema::{
    clear_active_user, default_root_openhuman_dir, pre_login_user_dir, read_active_user_id,
    user_openhuman_dir, write_active_user_id, PRE_LOGIN_USER_ID,
};
pub use schemas::{
    all_controller_schemas as all_config_controller_schemas,
    all_registered_controllers as all_config_registered_controllers,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexported_config_default_is_constructible() {
        let config = Config::default();

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
            channel_id: None,
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
