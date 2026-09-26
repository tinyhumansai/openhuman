# schema

Defines the `Config` struct — the single source of truth for `config.toml` —
and everything needed to load, save, and migrate it. AGENTS.md points
contributors here: "Rust configuration is defined under
`crates/openhuman-core/src/config/schema/` and loaded through its config
operations" (the operations live in `../ops/`, re-exported as `config::rpc`).

`Config` itself is split into submodules: `types.rs` declares `types/config.rs`
(the struct definition), `types/model_ids.rs` (model constants), and
`types/defaults.rs` / `types/output_language.rs` / `types/resolvers.rs` (small
helper types) purely to keep any one file under the repo's ~500-line
guideline; treat them as one unit. `load_user_state.rs` sits at this level but
is mounted as a submodule of `load/dirs.rs` via `#[path]`.

## Layout — `[section]` → file → struct

| `config.toml` section | File | Struct |
| --- | --- | --- |
| `[agent]`, `[orchestrator]`, `[teams.*]`, `[agents.*]` | `agent.rs` | `AgentConfig`, `OrchestratorModelConfig`, `TeamModelConfig`, `DelegateAgentConfig` |
| `agent_activity_level` (scalar key) | `activity_level.rs` | `AgentActivityLevel` |
| `[autonomy]` | `autonomy.rs` | `AutonomyConfig` — feeds `security::SecurityPolicy` |
| `[[capability_providers]]` | `capability_providers.rs` | `CapabilityProviderConfig` |
| `[channels_config]`, `[sandbox]` | `channels.rs` | `ChannelsConfig` and per-provider configs re-exported from `tinychannels_bus`; `SandboxConfig`, `ResourceLimitsConfig`, `AuditConfig`, and `SecurityConfig` (only `DaemonConfig` embeds the last one) |
| `[claude_agent_sdk]` | `claude_agent_sdk.rs` | `ClaudeAgentSdkConfig` |
| `[[cloud_providers]]`, `primary_cloud`, `*_provider` slugs | `cloud_providers.rs` | `CloudProviderCreds`, `CloudProviderType`, `AuthStyle` |
| `[context]` | `context.rs` | `ContextConfig` |
| `[cost]` | `identity_cost.rs` | `CostConfig`, `ModelPricing` |
| `[dashboard]` | `dashboard.rs` | `DashboardConfig`, `DiagramViewerConfig`, `EventStreamConfig`, `ModelHealthConfig` |
| `[dictation]` | `dictation.rs` | `DictationConfig`, `DictationActivationMode` |
| `[ephemeral_route]` | `ephemeral_route.rs` | `EphemeralRoute` |
| `[heartbeat]`, `[cron]` | `heartbeat_cron.rs` | `HeartbeatConfig`, `CronConfig`, `SubconsciousMode` |
| `[hooks]` | `hooks.rs` | `HooksConfig` |
| `[hosting]` | `hosting.rs` | `HostingConfig` |
| `[learning]` | `learning.rs` | `LearningConfig`, `ReflectionSource` |
| `[local_ai]` | `local_ai.rs` | `LocalAiConfig`, `LocalAiUsage` |
| `[modules]` | `modules.rs` | `ModulesConfig`, `ModuleOverride` — controls only whether compiled-in modules load; the loadable *set* is fixed by `crate::modules::registry` |
| `[node]` | `node.rs` | `NodeConfig` (managed Node.js toolchain for skills) |
| `[observability]` | `observability.rs` | `ObservabilityConfig`, `AgentTracingConfig` |
| `[privacy]` | `privacy.rs` | `PrivacyConfig`, `PrivacyMode` |
| `[proxy]` | `proxy.rs` | `ProxyConfig`, `ProxyScope`, plus `runtime_proxy_config()` / `set_runtime_proxy_config()` process-wide accessors |
| `[[model_routes]]`, `[[embedding_routes]]` | `routes.rs` | `ModelRouteConfig`, `EmbeddingRouteConfig` |
| `[runtime]` (+ `[runtime.docker]`), `[shell]`, `[reliability]`, `[scheduler]` | `runtime.rs` | `RuntimeConfig`, `DockerRuntimeConfig`, `ShellConfig`, `ReliabilityConfig`, `SchedulerConfig` |
| `[runtime_pool]` | `runtime_pool.rs` | `RuntimePoolConfig`, `RuntimePoolLangConfig` |
| `[runtime_python]` | `runtime_python.rs` | `RuntimePythonConfig` |
| `[scheduler_gate]` | `scheduler_gate.rs` | `SchedulerGateConfig`, `SchedulerGateMode` |
| `[memory]`, `[memory_tree]`, `[storage]` | `storage_memory.rs` | `MemoryConfig`, `MemoryTreeConfig`, `StorageConfig`, `StorageProviderConfig`, `LlmBackend` |
| `[subconscious]` | `subconscious.rs` | `SubconsciousConfig`, `SubconsciousEngine` (`local`) |
| `[subsystems]` | `subsystems.rs` | `SubsystemsConfig`, `MemorySubsystemConfig` |
| `[task_sources]` | `task_sources.rs` | `TaskSourcesConfig` |
| `[tokenjuice]` | `tokenjuice.rs` | `TokenjuiceConfig` |
| tool-related sections (see below) | `tools/` | — |
| `[update]` | `update.rs` | `UpdateConfig`, `UpdateRestartStrategy` |
| `[voice_server]` | `voice_server.rs` | `VoiceServerConfig`, `SttEngine`, `VoiceActivationMode` |
| `[[voice_providers]]`, `stt_provider` / `tts_provider` | `voice_providers.rs` | `VoiceProviderCreds`, `BuiltinVoiceProvider`, `BUILTIN_VOICE_PROVIDERS` |
| top-level `Config` and its scalar keys | `types.rs` (+ `types/config.rs`, `types/model_ids.rs`) | `Config`, `ModelRegistryEntry`, `MODEL_*` / `DEFAULT_MODEL` constants |
| built-in defaults | `defaults.rs` | `impl Default for Config` and per-field `default_*` fns |

`tools/mod.rs` groups the tool-facing sections: `browser.rs` (`BrowserConfig`,
`BrowserComputerUseConfig`), `http.rs` (`HttpRequestConfig`, `CurlConfig`),
`integrations.rs` (`IntegrationsConfig`, `ComposioConfig`, `SecretsConfig`),
`mcp.rs` (`McpServerConfig`, `McpClientConfig`, `GitbooksConfig`),
`multimodal.rs` (`MultimodalConfig`, `MultimodalFileConfig`), `search.rs`
(`SearchConfig`, `WebSearchConfig`, `SearxngConfig`, `SeltzConfig`).
`SearchConfig.enabled_providers` is an explicit provider set. When omitted,
legacy settings enable providers with saved keys, the managed backend when a
credential is available, and the separate TinyFish/Seltz/SearXNG toggles.
`engine = "disabled"` still disables search. TinySearch uses one route per
provider: explicit direct Parallel wins over managed backend Parallel when
both are selected.

Most sections have a matching `*_tests.rs` (some further split into several
`*_tests.rs` siblings, e.g. `types_model_pin_tests.rs`); this is the repo's
file-size-splitting convention, not separate modules.

## Loading

`Config::load_or_init` (in `load/impl_load.rs`) is the entry point used by
`config::ops::loader::load_config_with_timeout`. For an existing
`config.toml` it runs, in this order:

1. Resolve the config and workspace directories (`load/dirs.rs`,
   `resolve_runtime_config_dirs_with`). With no config file and the default
   config dir, it returns a pre-login in-memory `Config` without writing
   anything.
2. Read and parse `config.toml` (`parse_config_with_recovery`), recovering
   from non-UTF-8 or unparsable content by renaming the file to
   `.corrupted[.<ts>]`, trying the `.bak` copy, and finally falling back to
   defaults with `recovered_from_corruption = true`.
3. Fill `config_path` / `workspace_dir` / `action_dir` (`resolve_action_dir`),
   then apply the two pre-schema-version legacy rewrites in `load/migrate.rs`
   (`migrate_legacy_inference_url`, `migrate_cloud_provider_slugs`).
4. Apply environment-variable overrides — `Config::apply_env_overrides_from`
   in `load/env_overlay.rs`, split into submodules under `load/env_overlay/`
   (`dictation_context.rs`, `learning_memory.rs`, `observability.rs`,
   `proxy.rs`, `runtime.rs`, `search.rs`, `subsystems_update.rs`);
   `load/env.rs` is only the `EnvLookup` seam that lets tests supply a fake
   environment.
5. Run pending schema migrations (`../migrations/`, via
   `migrations::run_pending`), which bump `schema_version` and save as they go.
6. Decrypt at-rest secrets (`load/secrets.rs`); a legacy `enc:` (XOR) value is
   force-migrated to `enc2:` (ChaCha20-Poly1305 via `security::keyring`) and
   the config is saved again immediately.

A brand-new workspace is instead stamped at `CURRENT_SCHEMA_VERSION`, seeded
via `migrations::seed_new_workspace`, saved (0600 on Unix), and then given the
same env overlay.

`Config::save()` restores CLI-overridden inference fields
(`cli_overrides::restore_persisted_inference_fields`), re-encrypts secrets,
writes and fsyncs a 0600 temp file, then hands off to
`load/atomic_commit.rs::commit_replacement`, which preserves the previous
config as `.bak` and renames the temp file into place. The split exists so
callers can tell "nothing written" (an `Err` before the rename) from "already
committed" (after it) and roll back in-memory state accordingly — the
migrations runner depends on this.

`load/mod.rs` also exports `CONFIG_OWNER_MISMATCH_MARKER`: the loader appends
this marker to a config-read failure when the file's owner differs from the
reading process, and `core::observability::expected_error_kind` keys on it to
keep that failure paging instead of being demoted as ordinary user-environment
state.

## Workspace / identity helpers (`load/dirs.rs`)

Re-exported through `config::mod` and `config::schema::mod`:

- `default_root_openhuman_dir`, `user_openhuman_dir` — the per-user
  `~/.openhuman/users/<user-id>/` root.
- `resolve_action_dir`, `default_action_dir`, `action_dir_env_override`
  (`OPENHUMAN_ACTION_DIR`) — the agent's sandboxed read/write root.
- `active_workspace_dir` / `active_workspace_dir_cached` — resolve (and
  synchronously cache, via `load/active_workspace.rs`) the workspace the
  loader last resolved, for callers (like the developer Event Log's SSE
  stream) that cannot afford a disk read per lookup.
- `PRE_LOGIN_USER_ID` (`"local"`), `pre_login_user_dir`,
  `read_active_user_id` / `write_active_user_id` / `clear_active_user` — the
  pre-authentication identity scope and the `active_user.toml` marker
  (implemented in `../load_user_state.rs`).

`config` only *describes* these roots. Per AGENTS.md: `action_dir` is the
agent's permitted read/write root, and `workspace_dir` stores internal state
and is never an acting-tool target — enforcement of that boundary lives in
`security::SecurityPolicy` (`security/policy/`), not here. `autonomy.rs`
(`AutonomyConfig`) is the config-side half of the same contract: it is read
into `SecurityPolicy` at startup and on every settings change
(`config::ops::agent::apply_autonomy_settings` calls
`security::live_policy::reload_from`).

## `cli_overrides/`

Process-local inference overrides supplied by the standalone CLI
(`set_cli_inference_overrides`, `apply_cli_inference_overrides`,
`restore_persisted_inference_fields` — all `pub(crate)` — and the
`#[doc(hidden)]` `AppliedInferenceOverride` snapshot stored on
`Config::cli_inference_snapshot`) — lets a CLI invocation temporarily swap
model/provider without the override ever reaching the persisted config.

## Tests

Per-section `*_tests.rs` files, plus `load_tests.rs` (split into
`load_active_user_and_dirs_tests.rs`, `load_env_overlay_tests.rs`,
`load_corruption_recovery_tests.rs`, `load_migration_tests.rs`,
`load_backup_tests.rs`) for the loader/env-override/migration surface.

## Related docs

- [../README.md](../README.md) — the `config` module overview.
- [../ops/README.md](../ops/README.md) — the mutation/RPC surface built on
  top of this schema.
- [../migrations/README.md](../migrations/README.md) — automatic
  schema-version upgrades run during load.
- [../../security/README.md](../../security/README.md) — enforces the
  `action_dir` / `workspace_dir` boundary this module only describes.
