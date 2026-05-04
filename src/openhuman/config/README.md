# Config

Authoritative TOML-backed configuration layer. Owns the `Config` schema (every domain section: agent, channels, memory, autonomy, voice, scheduler, observability, etc.), env-variable overrides, the per-user openhuman directory layout, runtime proxy settings, the daemon descriptor, and the settings CLI. Roughly 177 internal consumers — almost every other domain reads `Config` here.

## Public surface

- `pub struct Config` — `schema/types.rs` (re-exported `mod.rs:28`) — top-level user settings.
- Per-domain config structs (re-exported `mod.rs:28-39`): `AgentConfig`, `AuditConfig`, `AutocompleteConfig`, `AutonomyConfig`, `BrowserComputerUseConfig`, `BrowserConfig`, `ChannelsConfig`, `ComposioConfig`, `ContextConfig`, `CostConfig`, `CronConfig`, `CurlConfig`, `DelegateAgentConfig`, `DictationConfig`, `DiscordConfig`, `DockerRuntimeConfig`, `EmbeddingRouteConfig`, `GitbooksConfig`, `HeartbeatConfig`, `HttpRequestConfig`, `IMessageConfig`, `IntegrationsConfig`, `LarkConfig`, `LearningConfig`, `LocalAiConfig`, `MatrixConfig`, `MemoryConfig`, `ModelRouteConfig`, `MultimodalConfig`, `ObservabilityConfig`, `ProxyConfig`, `ReliabilityConfig`, `ResourceLimitsConfig`, `RuntimeConfig`, `SandboxConfig`, `SchedulerConfig`, `ScreenIntelligenceConfig`, `SecretsConfig`, `SecurityConfig`, `SlackConfig`, `StorageConfig`, `TelegramConfig`, `UpdateConfig`, `VoiceServerConfig`, `WebSearchConfig`, `WebhookConfig`.
- Enums: `DictationActivationMode`, `IntegrationToggle`, `ProxyScope`, `ReflectionSource`, `SandboxBackend`, `StorageProviderConfig`, `StorageProviderSection`, `StreamMode`, `VoiceActivationMode`.
- Model constants: `DEFAULT_MODEL`, `MODEL_AGENTIC_V1`, `MODEL_CODING_V1`, `MODEL_REASONING_V1`.
- `pub struct DaemonConfig` — `daemon.rs` — sidecar lifecycle / port descriptor.
- `pub fn apply_runtime_proxy_to_builder` / `pub fn build_runtime_proxy_client` / `pub fn build_runtime_proxy_client_with_timeouts` / `pub fn runtime_proxy_config` / `pub fn set_runtime_proxy_config` — `schema/proxy.rs`.
- Workspace identity helpers: `pub fn clear_active_user`, `default_root_openhuman_dir`, `pre_login_user_dir`, `read_active_user_id`, `user_openhuman_dir`, `write_active_user_id`, `PRE_LOGIN_USER_ID` — `schema/identity_cost.rs`.
- `pub mod ops` (re-exported as `rpc`) — `ops.rs` — RPC handlers and settings mutation.
- `pub mod settings_cli` — `settings_cli.rs` — `openhuman settings ...` CLI surface.
- RPC `config.{get_config, update_model_settings, update_memory_settings, update_screen_intelligence_settings, update_runtime_settings, update_browser_settings, resolve_api_url, get_runtime_flags, set_browser_allow_all, workspace_onboarding_flag_exists, workspace_onboarding_flag_set, update_analytics_settings, get_analytics_settings, agent_server_status, reset_local_data, get_onboarding_completed, get_dictation_settings, update_dictation_settings, get_voice_server_settings, update_voice_server_settings, set_onboarding_completed}` — `schemas.rs`.

## Calls into

- Std + serde TOML for serialization.
- `src/openhuman/encryption/` indirectly when secrets sections need at-rest crypto (read direction only).
- Filesystem under `~/.openhuman/<user-id>/` via `schema/identity_cost.rs`.

## Called by

- ~177 sites across the workspace — every domain pulls `Config` for its slice.
- Hot consumers: `src/openhuman/agent/` (model + autonomy), `src/openhuman/channels/` (provider tokens), `src/openhuman/memory/` (storage paths), `src/openhuman/cron/` (scheduler poll), `src/openhuman/local_ai/` (Ollama / device routing), `src/openhuman/security/` (sandbox backend), `src/openhuman/voice/`, `src/openhuman/notifications/`, `src/openhuman/tools/`, `src/openhuman/encryption/`, `src/openhuman/tree_summarizer/`, `src/openhuman/referral/`.
- `src/core/all.rs` — registers `all_config_*`.

## Tests

- Unit: `ops_tests.rs`, `schemas_tests.rs`, plus per-section `*_tests.rs` under `schema/` (`channels_tests.rs`, `load_tests.rs`, `proxy_tests.rs`).
- Cross-test serialization: `schema/load.rs` round-trips against `schema/defaults.rs`.
- `TEST_ENV_LOCK` (`mod.rs:55`) is shared with sibling test modules that mutate `OPENHUMAN_WORKSPACE`.
