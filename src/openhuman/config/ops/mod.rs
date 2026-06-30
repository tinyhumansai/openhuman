//! JSON-RPC / CLI controller surface for persisted config and runtime flags.

mod agent;
mod loader;
mod model;
mod sandbox;
mod ui;

// ── Public re-exports (preserving the flat external API) ─────────────────────

pub use agent::redact_home;
pub use agent::{
    add_auto_approve_tool, apply_activity_level_settings, apply_agent_paths_settings,
    apply_agent_settings, apply_autonomy_settings, apply_memory_sync_settings, ensure_agent_dirs,
    ensure_usable_cwd, expand_tilde, get_activity_level_settings, get_agent_paths,
    get_agent_settings, get_autonomy_settings, get_memory_sync_settings,
    load_and_apply_activity_level_settings, load_and_apply_agent_paths_settings,
    load_and_apply_agent_settings, load_and_apply_autonomy_settings,
    load_and_apply_memory_sync_settings, ActivityLevelSettingsPatch, AgentPathsPatch,
    AgentSettingsPatch, AutonomySettingsPatch, MemorySyncSettingsPatch,
};

pub use loader::{
    agent_server_status, client_config_json, core_rpc_url_from_env, get_config_snapshot,
    get_dashboard_settings, get_data_paths, get_runtime_flags, load_and_get_client_config_snapshot,
    load_and_get_config_snapshot, load_config_with_timeout, reload_config_snapshot_with_timeout,
    reset_local_data, set_browser_allow_all, snapshot_config_json, RuntimeFlagsOut,
};
// expose internal helpers needed by tests (ops_tests.rs uses super::*)
#[cfg(test)]
pub(crate) use crate::openhuman::config::Config;
#[cfg(test)]
pub(crate) use loader::{
    active_workspace_marker_path, config_openhuman_dir, default_openhuman_dir, env_flag_enabled,
    fallback_workspace_dir, reset_local_data_for_paths, reset_local_data_remove_error,
    BROWSER_ALLOW_ALL_ENV, BROWSER_ALLOW_ALL_RPC_ENABLE_ENV,
};
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

pub use sandbox::{
    apply_sandbox_settings, get_sandbox_settings, load_and_apply_sandbox_settings,
    SandboxSettingsPatch,
};

pub use ui::{
    apply_analytics_settings, apply_browser_settings, apply_meet_settings,
    apply_screen_intelligence_settings, apply_search_settings, get_dictation_settings,
    get_onboarding_completed, get_search_settings, get_super_context_enabled,
    get_voice_server_settings, load_and_apply_analytics_settings, load_and_apply_browser_settings,
    load_and_apply_dictation_settings, load_and_apply_meet_settings,
    load_and_apply_screen_intelligence_settings, load_and_apply_search_settings,
    load_and_apply_voice_server_settings, set_onboarding_completed, set_super_context_enabled,
    workspace_onboarding_flag_exists, workspace_onboarding_flag_resolve,
    workspace_onboarding_flag_set, AnalyticsSettingsPatch, BrowserSettingsPatch,
    DictationSettingsPatch, MeetSettingsPatch, ScreenIntelligenceSettingsPatch,
    SearchSettingsPatch, VoiceServerSettingsPatch,
};

#[cfg(test)]
#[path = "../ops_tests.rs"]
mod tests;
