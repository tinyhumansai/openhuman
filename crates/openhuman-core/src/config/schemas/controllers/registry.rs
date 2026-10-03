//! Controller registry for the `config` namespace: schema list and handler wiring.

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;

use super::super::schema_defs::schemas;
use super::agent::{
    handle_get_agent_settings, handle_get_autonomy_settings, handle_get_privacy_mode,
    handle_get_sandbox_settings, handle_set_browser_allow_all, handle_set_privacy_mode,
    handle_update_agent_settings, handle_update_autonomy_settings, handle_update_browser_settings,
    handle_update_computer_settings, handle_update_sandbox_settings,
};
use super::inference::{
    handle_get_client_config, handle_get_config, handle_get_runtime_flags, handle_resolve_api_url,
    handle_update_local_ai_settings, handle_update_memory_settings, handle_update_model_settings,
    handle_update_runtime_settings,
};
use super::integrations::{
    handle_get_composio_trigger_settings, handle_get_search_settings,
    handle_update_composio_trigger_settings, handle_update_search_settings,
};
use super::voice::{
    handle_get_dictation_settings, handle_get_voice_server_settings,
    handle_update_dictation_settings, handle_update_voice_server_settings,
};
use super::workspace::{
    handle_agent_server_status, handle_get_agent_paths, handle_get_analytics_settings,
    handle_get_dashboard_settings, handle_get_data_paths, handle_get_onboarding_completed,
    handle_reset_local_data, handle_set_onboarding_completed, handle_update_agent_paths,
    handle_update_analytics_settings, handle_workspace_onboarding_flag_exists,
    handle_workspace_onboarding_flag_set,
};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("get_config"),
        schemas("get_client_config"),
        schemas("update_model_settings"),
        schemas("update_memory_settings"),
        schemas("update_runtime_settings"),
        schemas("update_browser_settings"),
        schemas("update_computer_settings"),
        schemas("update_local_ai_settings"),
        schemas("resolve_api_url"),
        schemas("get_runtime_flags"),
        schemas("set_browser_allow_all"),
        schemas("workspace_onboarding_flag_exists"),
        schemas("workspace_onboarding_flag_set"),
        schemas("update_analytics_settings"),
        schemas("get_analytics_settings"),
        schemas("get_dashboard_settings"),
        schemas("agent_server_status"),
        schemas("reset_local_data"),
        schemas("get_data_paths"),
        schemas("get_agent_paths"),
        schemas("update_agent_paths"),
        schemas("get_onboarding_completed"),
        schemas("set_onboarding_completed"),
        schemas("get_dictation_settings"),
        schemas("update_dictation_settings"),
        schemas("get_voice_server_settings"),
        schemas("update_voice_server_settings"),
        schemas("update_composio_trigger_settings"),
        schemas("get_composio_trigger_settings"),
        schemas("get_autonomy_settings"),
        schemas("update_autonomy_settings"),
        schemas("get_privacy_mode"),
        schemas("set_privacy_mode"),
        schemas("get_agent_settings"),
        schemas("update_agent_settings"),
        schemas("update_search_settings"),
        schemas("get_search_settings"),
        schemas("get_sandbox_settings"),
        schemas("update_sandbox_settings"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("get_config"),
            handler: handle_get_config,
        },
        RegisteredController {
            schema: schemas("get_client_config"),
            handler: handle_get_client_config,
        },
        RegisteredController {
            schema: schemas("update_model_settings"),
            handler: handle_update_model_settings,
        },
        RegisteredController {
            schema: schemas("update_memory_settings"),
            handler: handle_update_memory_settings,
        },
        RegisteredController {
            schema: schemas("update_runtime_settings"),
            handler: handle_update_runtime_settings,
        },
        RegisteredController {
            schema: schemas("update_browser_settings"),
            handler: handle_update_browser_settings,
        },
        RegisteredController {
            schema: schemas("update_computer_settings"),
            handler: handle_update_computer_settings,
        },
        RegisteredController {
            schema: schemas("update_local_ai_settings"),
            handler: handle_update_local_ai_settings,
        },
        RegisteredController {
            schema: schemas("resolve_api_url"),
            handler: handle_resolve_api_url,
        },
        RegisteredController {
            schema: schemas("get_runtime_flags"),
            handler: handle_get_runtime_flags,
        },
        RegisteredController {
            schema: schemas("set_browser_allow_all"),
            handler: handle_set_browser_allow_all,
        },
        RegisteredController {
            schema: schemas("workspace_onboarding_flag_exists"),
            handler: handle_workspace_onboarding_flag_exists,
        },
        RegisteredController {
            schema: schemas("workspace_onboarding_flag_set"),
            handler: handle_workspace_onboarding_flag_set,
        },
        RegisteredController {
            schema: schemas("update_analytics_settings"),
            handler: handle_update_analytics_settings,
        },
        RegisteredController {
            schema: schemas("get_analytics_settings"),
            handler: handle_get_analytics_settings,
        },
        RegisteredController {
            schema: schemas("get_dashboard_settings"),
            handler: handle_get_dashboard_settings,
        },
        RegisteredController {
            schema: schemas("agent_server_status"),
            handler: handle_agent_server_status,
        },
        RegisteredController {
            schema: schemas("reset_local_data"),
            handler: handle_reset_local_data,
        },
        RegisteredController {
            schema: schemas("get_data_paths"),
            handler: handle_get_data_paths,
        },
        RegisteredController {
            schema: schemas("get_agent_paths"),
            handler: handle_get_agent_paths,
        },
        RegisteredController {
            schema: schemas("update_agent_paths"),
            handler: handle_update_agent_paths,
        },
        RegisteredController {
            schema: schemas("get_onboarding_completed"),
            handler: handle_get_onboarding_completed,
        },
        RegisteredController {
            schema: schemas("set_onboarding_completed"),
            handler: handle_set_onboarding_completed,
        },
        RegisteredController {
            schema: schemas("get_dictation_settings"),
            handler: handle_get_dictation_settings,
        },
        RegisteredController {
            schema: schemas("update_dictation_settings"),
            handler: handle_update_dictation_settings,
        },
        RegisteredController {
            schema: schemas("get_voice_server_settings"),
            handler: handle_get_voice_server_settings,
        },
        RegisteredController {
            schema: schemas("update_voice_server_settings"),
            handler: handle_update_voice_server_settings,
        },
        RegisteredController {
            schema: schemas("update_composio_trigger_settings"),
            handler: handle_update_composio_trigger_settings,
        },
        RegisteredController {
            schema: schemas("get_composio_trigger_settings"),
            handler: handle_get_composio_trigger_settings,
        },
        RegisteredController {
            schema: schemas("get_autonomy_settings"),
            handler: handle_get_autonomy_settings,
        },
        RegisteredController {
            schema: schemas("update_autonomy_settings"),
            handler: handle_update_autonomy_settings,
        },
        RegisteredController {
            schema: schemas("get_privacy_mode"),
            handler: handle_get_privacy_mode,
        },
        RegisteredController {
            schema: schemas("set_privacy_mode"),
            handler: handle_set_privacy_mode,
        },
        RegisteredController {
            schema: schemas("get_agent_settings"),
            handler: handle_get_agent_settings,
        },
        RegisteredController {
            schema: schemas("update_agent_settings"),
            handler: handle_update_agent_settings,
        },
        RegisteredController {
            schema: schemas("update_search_settings"),
            handler: handle_update_search_settings,
        },
        RegisteredController {
            schema: schemas("get_search_settings"),
            handler: handle_get_search_settings,
        },
        RegisteredController {
            schema: schemas("get_sandbox_settings"),
            handler: handle_get_sandbox_settings,
        },
        RegisteredController {
            schema: schemas("update_sandbox_settings"),
            handler: handle_update_sandbox_settings,
        },
    ]
}
