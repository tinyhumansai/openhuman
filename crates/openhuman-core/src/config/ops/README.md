# ops

JSON-RPC / CLI controller surface for persisted config and runtime flags: the
mutation half of `config`. `crate::config` re-exports this module both under
its own name and as `rpc` (`pub use ops as rpc`), so most callers write
`config::rpc::*`. Controllers in `../schemas/` are thin wrappers around the
functions here: they deserialize RPC params into `../schemas/helpers.rs`
`*SettingsUpdate` structs, map those field-by-field onto the `*SettingsPatch`
structs defined here, and call the corresponding `load_and_apply_*` / `get_*`
fn, which returns `Outcome<T>`.

## Layout

| File | Responsibility |
| --- | --- |
| `agent.rs` | Autonomy, agent, agent-paths, and memory-sync settings. |
| `loader.rs` | Config loading/snapshotting and runtime flags; split into submodules `loader/load.rs`, `loader/paths.rs`, `loader/reset_local_data.rs`, `loader/runtime_flags.rs`, `loader/snapshot.rs`. |
| `model.rs` | AI-provider, memory, runtime, local-AI, and Composio-trigger settings. |
| `privacy.rs` | Privacy Mode (`[privacy]`) get/set. |
| `sandbox.rs` | Sandbox / Docker runtime (`[sandbox]`, `[runtime.docker]`) settings. |
| `ui.rs` | Browser, analytics, dictation, voice-server, and onboarding-flag settings. |
| `search.rs` | Search settings: providers, routes, role order, keys, limits, and the web-access allowlist. |

Each submodule follows the same shape: a `*SettingsPatch` struct with
`Option<T>` fields (`None` = unchanged); an `apply_*(&mut Config, patch)` fn
that mutates the given config, calls `Config::save()`, and returns the
settings or snapshot; a `load_and_apply_*(patch)` wrapper that calls
`load_config_with_timeout` first; and a `get_*` fn that reads the relevant
section back out, usually as `Outcome<serde_json::Value>`. `ui.rs`'s
dictation and voice-server mutators exist only in `load_and_apply_*` form.

## Key entry points

- `agent.rs`: `apply_autonomy_settings` / `get_autonomy_settings`,
  `add_auto_approve_tool`, `apply_agent_settings` / `get_agent_settings`,
  `apply_agent_paths_settings` / `get_agent_paths`, `ensure_usable_cwd`,
  `expand_tilde`, `redact_home`.
- `loader.rs`: `load_config_with_timeout`,
  `load_config_for_workspace_with_timeout`, `get_config_snapshot`,
  `client_config_json`, `reload_config_from_paths`, `reset_local_data`,
  `get_data_paths`, `set_browser_allow_all`, `get_runtime_flags`,
  `core_rpc_url_from_env`, `agent_server_status`, `get_dashboard_settings`.
  `BROWSER_ALLOW_ALL_ENV` (`OPENHUMAN_BROWSER_ALLOW_ALL`) and
  `BROWSER_ALLOW_ALL_RPC_ENABLE_ENV` are `pub(crate)` constants re-exported
  from `mod.rs` only under `#[cfg(test)]`.
- `model.rs`: `apply_model_settings`, `apply_memory_settings`,
  `apply_runtime_settings`, `apply_local_ai_settings`,
  `apply_composio_trigger_settings`, `load_and_resolve_api_url`.
- `privacy.rs`: `apply_privacy_settings`, `get_privacy_mode`.
- `sandbox.rs`: `apply_sandbox_settings`, `get_sandbox_settings`.
- `search.rs`: `apply_search_settings` / `get_search_settings`,
  `search_settings_json`.
- `ui.rs`: `apply_browser_settings`, `apply_analytics_settings`,
  `load_and_apply_voice_server_settings` / `get_voice_server_settings`,
  `load_and_apply_dictation_settings` / `get_dictation_settings`,
  `set_onboarding_completed` / `get_onboarding_completed`,
  `workspace_onboarding_flag_exists` / `workspace_onboarding_flag_set` /
  `workspace_onboarding_flag_resolve`.

## Search settings

`apply_search_settings` validates the whole patch before saving: a global
`enabled` switch; per-provider `{enabled, route, api_key, base_url}` (routes a
provider does not support are rejected, an empty key clears it); per-role
provider order (`search`, `answer`, `contents`; a provider that cannot serve a
role is rejected, an empty list restores the default); presentation; limits;
and the web-access allowlist. The legacy `engine` field from older clients is
still accepted (`SearchConfig::apply_legacy_engine`). The response and
`get_search_settings` return the same view: each provider's resolved route,
key presence, `usable` and status, the configured role order, and
`effective_roles` (what serves each role right now). Keys are never returned.
After saving, a loaded TinySearch module is refreshed privately.

## Security-relevant behavior

- `apply_autonomy_settings` is the single write path for `[autonomy]`: after
  `Config::save()` it calls
  `crate::security::live_policy::reload_from(&config.autonomy)` and publishes
  `DomainEvent::AutonomyConfigChanged`, so the live `SecurityPolicy` and any
  running agent sessions pick up the change without a core restart.
  `add_auto_approve_tool` (backing the `ApproveAlwaysForTool` approval
  decision) appends to `config.autonomy.auto_approve` under a process-wide
  lock and delegates to `apply_autonomy_settings`, so the same reload
  happens. `apply_agent_paths_settings` calls
  `crate::security::live_policy::set_action_dir` when `action_dir` changes.
  Do not weaken these settings mutators; they gate the same autonomy
  invariants AGENTS.md requires of `security/`.
- `apply_privacy_settings` calls `crate::security::live_policy::reload_privacy`
  after saving, so the inference chokepoint enforces the new Privacy Mode
  immediately (an `Err` only means no session runtime is installed yet, e.g.
  the CLI, and the persisted value applies on the next install).
- `reset_local_data` (RPC `config.reset_local_data`) removes only the active
  user's `~/.openhuman/users/<id>` directory plus the two root markers
  (`active_workspace.toml`, `active_user.toml`); the shared root is preserved
  so sibling users' data survives. Because it runs inside the core's own tokio
  task, GUI callers use the Tauri-side `reset_local_data` command instead
  (which stops the core first); `get_data_paths` reports what would be removed
  without touching anything.

## `#[cfg(test)]` re-exports

`mod.rs` re-exports several otherwise-private items (`Config`,
`active_workspace_marker_path`, `resolve_backend_api_url`, etc.) behind
`#[cfg(test)]` purely so `ops_tests.rs` and its sibling test files
(`ops_agent_paths_tests.rs`, `ops_loader_and_search_tests.rs`,
`ops_model_and_local_ai_tests.rs`, `ops_voice_and_autonomy_tests.rs`) can reach
them through `use super::*`; they carry no runtime meaning outside test
builds.
