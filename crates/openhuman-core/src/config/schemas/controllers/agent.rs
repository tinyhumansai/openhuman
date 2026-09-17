//! Handlers for agent behaviour settings: autonomy, privacy, browser, sandbox, activity level, and memory sync.

use serde_json::{Map, Value};

use crate::config::rpc as config_rpc;
use crate::core::all::ControllerFuture;

use super::super::helpers::{
    deserialize_params, to_json, ActivityLevelSettingsUpdate, AgentSettingsUpdate,
    AutonomySettingsUpdate, BrowserSettingsUpdate, MemorySyncSettingsUpdate, PrivacyModeUpdate,
    SandboxSettingsUpdate, SetBrowserAllowAllParams,
};

pub(crate) fn handle_get_autonomy_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(config_rpc::get_autonomy_settings().await?) })
}

pub(crate) fn handle_update_autonomy_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<AutonomySettingsUpdate>(params)?;
        let patch = config_rpc::AutonomySettingsPatch {
            level: update.level,
            workspace_only: update.workspace_only,
            allowed_commands: update.allowed_commands,
            forbidden_paths: update.forbidden_paths,
            trusted_roots: update.trusted_roots,
            allow_tool_install: update.allow_tool_install,
            max_actions_per_hour: update
                .max_actions_per_hour
                .map(|v| u32::try_from(v).unwrap_or(u32::MAX)),
            auto_approve: update.auto_approve,
            require_task_plan_approval: update.require_task_plan_approval,
            auto_approve_all: update.auto_approve_all,
        };
        to_json(config_rpc::load_and_apply_autonomy_settings(patch).await?)
    })
}

pub(super) fn handle_get_privacy_mode(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(config_rpc::get_privacy_mode().await?) })
}

pub(super) fn handle_set_privacy_mode(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<PrivacyModeUpdate>(params)?;
        let patch = config_rpc::PrivacySettingsPatch { mode: update.mode };
        to_json(config_rpc::load_and_apply_privacy_settings(patch).await?)
    })
}

pub(super) fn handle_get_agent_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        log::debug!("[config][rpc] get_agent_settings enter");
        match config_rpc::get_agent_settings().await {
            Ok(outcome) => {
                log::debug!("[config][rpc] get_agent_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] get_agent_settings failed: {err}");
                Err(err)
            }
        }
    })
}

pub(super) fn handle_update_agent_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] update_agent_settings enter");
        let update = match deserialize_params::<AgentSettingsUpdate>(params) {
            Ok(u) => u,
            Err(err) => {
                log::warn!("[config][rpc] update_agent_settings invalid params: {err}");
                return Err(err);
            }
        };
        let patch = config_rpc::AgentSettingsPatch {
            agent_timeout_secs: update.agent_timeout_secs,
            allow_metered_agent_tools: update.allow_metered_agent_tools,
        };
        match config_rpc::load_and_apply_agent_settings(patch).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] update_agent_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] update_agent_settings failed: {err}");
                Err(err)
            }
        }
    })
}

pub(super) fn handle_update_browser_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<BrowserSettingsUpdate>(params)?;
        let patch = config_rpc::BrowserSettingsPatch {
            enabled: update.enabled,
            backend: update.backend,
        };
        to_json(config_rpc::load_and_apply_browser_settings(patch).await?)
    })
}

pub(super) fn handle_set_browser_allow_all(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<SetBrowserAllowAllParams>(params)?;
        to_json(config_rpc::set_browser_allow_all(payload.enabled)?)
    })
}

pub(super) fn handle_get_activity_level_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(config_rpc::get_activity_level_settings().await?) })
}

pub(super) fn handle_update_activity_level_settings(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<ActivityLevelSettingsUpdate>(params)?;
        let patch = config_rpc::ActivityLevelSettingsPatch {
            level: update.level,
        };
        to_json(config_rpc::load_and_apply_activity_level_settings(patch).await?)
    })
}

pub(super) fn handle_get_memory_sync_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(config_rpc::get_memory_sync_settings().await?) })
}

pub(super) fn handle_update_memory_sync_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<MemorySyncSettingsUpdate>(params)?;
        let patch = config_rpc::MemorySyncSettingsPatch {
            sync_interval_secs: update.sync_interval_secs,
        };
        to_json(config_rpc::load_and_apply_memory_sync_settings(patch).await?)
    })
}

pub(super) fn handle_get_sandbox_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(config_rpc::get_sandbox_settings().await?) })
}

pub(super) fn handle_update_sandbox_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<SandboxSettingsUpdate>(params)?;
        let patch = config_rpc::SandboxSettingsPatch {
            backend: update.backend,
            enabled: update.enabled,
            docker_image: update.docker_image,
            docker_memory_limit_mb: update.docker_memory_limit_mb,
            docker_cpu_limit: update.docker_cpu_limit,
            env_passthrough: update.env_passthrough,
        };
        to_json(config_rpc::load_and_apply_sandbox_settings(patch).await?)
    })
}
