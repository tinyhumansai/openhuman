//! Handlers for agent behaviour settings: autonomy, privacy, browser, sandbox, and memory sync.

use serde_json::{Map, Value};

use crate::config::rpc as config_rpc;
use crate::core::all::ControllerFuture;

use super::super::helpers::{
    deserialize_params, to_json, AgentSettingsUpdate, AutonomySettingsUpdate,
    BrowserSettingsUpdate, ComputerSettingsUpdate, PrivacyModeUpdate, SandboxSettingsUpdate,
    SetBrowserAllowAllParams,
};

pub(crate) fn handle_get_autonomy_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(config_rpc::get_autonomy_settings().await?) })
}

pub(crate) fn handle_update_autonomy_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<AutonomySettingsUpdate>(params)?;
        let patch = config_rpc::AutonomySettingsPatch {
            enabled: update.enabled,
            level: update.level,
            workspace_only: update.workspace_only,
            allowed_commands: update.allowed_commands,
            forbidden_paths: update.forbidden_paths,
            trusted_roots: update.trusted_roots,
            allow_tool_install: update.allow_tool_install,
            max_actions_per_hour: update.max_actions_per_hour,
            auto_approve: update.auto_approve,
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
            chat_agent_id: update.chat_agent_id,
            tool_dispatcher: update.tool_dispatcher,
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
            headless: update.headless,
            viewport_width: update.viewport_width,
            viewport_height: update.viewport_height,
            chrome_path: update.chrome_path,
            profile_mode: update.profile_mode,
            profile_path: update.profile_path,
            download_dir: update.download_dir,
            max_task_steps: update.max_task_steps,
            task_timeout_secs: update.task_timeout_secs,
        };
        to_json(config_rpc::load_and_apply_browser_settings(patch).await?)
    })
}

pub(super) fn handle_update_computer_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<ComputerSettingsUpdate>(params)?;
        let patch = config_rpc::ComputerSettingsPatch {
            decision_model: update.decision_model,
            sage_fast: update.sage_fast,
            planner_model: update.planner_model,
            rescue_model: update.rescue_model,
            max_rescues: update.max_rescues,
        };
        to_json(config_rpc::load_and_apply_computer_settings(patch).await?)
    })
}

pub(super) fn handle_set_browser_allow_all(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<SetBrowserAllowAllParams>(params)?;
        to_json(config_rpc::set_browser_allow_all(payload.enabled)?)
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
