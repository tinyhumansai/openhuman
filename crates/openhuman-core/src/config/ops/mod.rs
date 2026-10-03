//! JSON-RPC / CLI controller surface for persisted config and runtime flags.

mod agent;
mod loader;
mod model;
mod privacy;
mod sandbox;
mod search;
mod tool_dispatcher;
mod ui;

// ── Public re-exports (preserving the flat external API) ─────────────────────

pub use agent::redact_home;
pub use agent::{
    add_auto_approve_tool, apply_agent_paths_settings, apply_agent_settings,
    apply_autonomy_settings, ensure_agent_dirs, ensure_usable_cwd, expand_tilde, get_agent_paths,
    get_agent_settings, get_autonomy_settings, load_and_apply_agent_paths_settings,
    load_and_apply_agent_settings, load_and_apply_autonomy_settings, AgentPathsPatch,
    AgentSettingsPatch, AutonomySettingsPatch,
};

pub use loader::{
    agent_server_status, client_config_json, core_rpc_url_from_env, get_config_snapshot,
    get_dashboard_settings, get_data_paths, get_data_paths_for_user, get_runtime_flags,
    load_and_get_client_config_snapshot, load_and_get_config_snapshot,
    load_config_for_workspace_with_timeout, load_config_with_timeout, reload_config_from_paths,
    reload_config_snapshot_with_timeout, reset_local_data, set_browser_allow_all,
    snapshot_config_json, RuntimeFlagsOut,
};
// expose internal helpers needed by tests (ops_tests.rs uses super::*)
#[cfg(test)]
pub(crate) use crate::config::Config;
#[cfg(all(test, windows))]
pub(crate) use loader::reset_local_data_remove_error;
#[cfg(test)]
pub(crate) use loader::{
    active_workspace_marker_path, config_openhuman_dir, default_openhuman_dir, env_flag_enabled,
    fallback_workspace_dir, reset_local_data_for_paths, BROWSER_ALLOW_ALL_ENV,
    BROWSER_ALLOW_ALL_RPC_ENABLE_ENV,
};
#[cfg(test)]
pub(crate) use model::resolve_backend_api_url;
#[cfg(test)]
pub(crate) use std::path::PathBuf;

pub use model::{
    apply_composio_trigger_settings, apply_local_ai_settings, apply_memory_settings,
    apply_model_settings, apply_runtime_settings, get_composio_trigger_settings,
    load_and_apply_composio_trigger_settings, load_and_apply_local_ai_settings,
    load_and_apply_memory_settings, load_and_apply_model_settings, load_and_apply_runtime_settings,
    load_and_resolve_api_url, ComposioTriggerSettingsPatch, LocalAiSettingsPatch,
    MemorySettingsPatch, ModelSettingsPatch, RuntimeSettingsPatch,
};

pub use privacy::{
    apply_privacy_settings, get_privacy_mode, load_and_apply_privacy_settings, PrivacySettingsPatch,
};

mod computer;
pub use computer::{
    apply_computer_settings, load_and_apply_computer_settings, ComputerSettingsPatch,
};

pub use sandbox::{
    apply_sandbox_settings, get_sandbox_settings, load_and_apply_sandbox_settings,
    SandboxSettingsPatch,
};

pub use search::{
    apply_search_settings, get_search_settings, load_and_apply_search_settings,
    search_settings_json, SearchProviderPatch, SearchSettingsPatch,
};
pub use ui::{
    apply_analytics_settings, apply_browser_settings, get_dictation_settings,
    get_onboarding_completed, get_voice_server_settings, load_and_apply_analytics_settings,
    load_and_apply_browser_settings, load_and_apply_dictation_settings,
    load_and_apply_voice_server_settings, set_onboarding_completed,
    workspace_onboarding_flag_exists, workspace_onboarding_flag_resolve,
    workspace_onboarding_flag_set, AnalyticsSettingsPatch, BrowserSettingsPatch,
    DictationSettingsPatch, VoiceServerSettingsPatch,
};

#[cfg(test)]
#[path = "../ops_tests.rs"]
mod tests;
