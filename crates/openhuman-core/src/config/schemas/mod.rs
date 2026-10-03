//! Controller schemas and thin RPC handlers for the `config` namespace.
//!
//! `all_controller_schemas` / `all_registered_controllers` are re-exported from
//! `config::mod` as `all_config_controller_schemas` / `all_config_registered_controllers`;
//! `core/all.rs` registers the latter under `DomainGroup::Config` (the schema-only
//! list is consumed by tests). Handlers here are thin: they deserialize params,
//! delegate to `config::ops` (re-exported as `config::rpc`) for the actual
//! mutation/read, and shape the `Outcome` response.
//!
//! - `controllers.rs` — declares submodules `controllers/{agent,inference,
//!   integrations,registry,voice,workspace}.rs` (split for file-size only;
//!   together they define every handler and the `all_controller_schemas` /
//!   `all_registered_controllers` lists, the latter in `controllers/registry.rs`).
//! - `helpers.rs` — param-deserialization update structs (`*SettingsUpdate`,
//!   `*Params`) and small JSON helpers (`deserialize_params`, `to_json`, etc.).
//! - `schema_defs.rs` — `schemas(function)`, the by-name `ControllerSchema`
//!   lookup, dispatching to the `schema_defs/{agent,inference,integrations,
//!   voice,workspace}.rs` submodules that hold the definitions.
//!
//! Tests: `../schemas_tests.rs` (mounted below) and `controllers_tests.rs`.
//!
//! Methods exposed under `config.*`: `get_config`, `get_client_config`,
//! `update_model_settings`, `update_memory_settings`, `update_runtime_settings`,
//! `update_browser_settings`, `update_local_ai_settings`, `resolve_api_url`,
//! `get_runtime_flags`, `set_browser_allow_all`, `workspace_onboarding_flag_exists`,
//! `workspace_onboarding_flag_set`, `update_analytics_settings`,
//! `get_analytics_settings`, `get_dashboard_settings`, `agent_server_status`,
//! `reset_local_data`, `get_data_paths`, `get_agent_paths`, `update_agent_paths`,
//! `get_onboarding_completed`, `set_onboarding_completed`, `get_dictation_settings`,
//! `update_dictation_settings`, `get_voice_server_settings`,
//! `update_voice_server_settings`, `update_composio_trigger_settings`,
//! `get_composio_trigger_settings`, `get_autonomy_settings`,
//! `update_autonomy_settings`, `get_privacy_mode`, `set_privacy_mode`,
//! `get_agent_settings`, `update_agent_settings`, `update_search_settings`,
//! `get_search_settings`, `get_sandbox_settings`,
//! `update_sandbox_settings`.

mod controllers;
mod helpers;
mod schema_defs;

pub use controllers::{all_controller_schemas, all_registered_controllers};

// Re-export items that schemas_tests.rs accesses via `use super::*`.
// The test module is `schemas::tests` so `super::` resolves to `schemas`.
#[cfg(test)]
use crate::core::Outcome;
#[cfg(test)]
use crate::core::TypeSchema;
#[cfg(test)]
use controllers::{
    handle_get_agent_paths, handle_get_autonomy_settings, handle_update_autonomy_settings,
};
#[cfg(test)]
use helpers::{
    deserialize_params, json_output, optional_bool, optional_json, optional_string,
    required_string, to_json, AutonomySettingsUpdate, LocalAiSettingsUpdate, MemorySettingsUpdate,
    ModelSettingsUpdate, OnboardingCompletedSetParams, SetBrowserAllowAllParams,
    WorkspaceOnboardingFlagParams, WorkspaceOnboardingFlagSetParams, DEFAULT_ONBOARDING_FLAG_NAME,
};
#[cfg(test)]
use serde_json::{Map, Value};

#[cfg(test)]
#[path = "../schemas_tests.rs"]
mod tests;
