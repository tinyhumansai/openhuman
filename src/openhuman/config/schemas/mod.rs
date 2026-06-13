mod controllers;
mod helpers;
mod schema_defs;

pub use controllers::{all_controller_schemas, all_registered_controllers};
pub use schema_defs::schemas;

// Re-export items that schemas_tests.rs accesses via `use super::*`.
// The test module is `schemas::tests` so `super::` resolves to `schemas`.
#[cfg(test)]
use crate::core::TypeSchema;
#[cfg(test)]
use crate::rpc::RpcOutcome;
#[cfg(test)]
use controllers::{
    handle_get_agent_paths, handle_get_autonomy_settings, handle_update_autonomy_settings,
};
#[cfg(test)]
use helpers::{
    deserialize_params, json_output, optional_bool, optional_json, optional_string,
    required_string, to_json, ActivityLevelSettingsUpdate, AgentPathsUpdate, AgentSettingsUpdate,
    AnalyticsSettingsUpdate, AutonomySettingsUpdate, BrowserSettingsUpdate,
    ComposioTriggerSettingsUpdate, DictationSettingsUpdate, LocalAiSettingsUpdate,
    MeetSettingsUpdate, MemorySettingsUpdate, MemorySyncSettingsUpdate, ModelSettingsUpdate,
    OnboardingCompletedSetParams, RuntimeSettingsUpdate, SandboxSettingsUpdate,
    ScreenIntelligenceSettingsUpdate, SearchSettingsUpdate, SetBrowserAllowAllParams,
    VoiceServerSettingsUpdate, WorkspaceOnboardingFlagParams, WorkspaceOnboardingFlagSetParams,
    DEFAULT_ONBOARDING_FLAG_NAME,
};
#[cfg(test)]
use serde_json::{Map, Value};

#[cfg(test)]
#[path = "../schemas_tests.rs"]
mod tests;
