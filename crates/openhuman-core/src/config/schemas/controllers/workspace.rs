//! Handlers for workspace state: onboarding flags, analytics, dashboard, data and agent paths, and local-data reset.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::config::rpc as config_rpc;
use crate::core::all::ControllerFuture;

use super::super::helpers::{
    deserialize_params, to_json, AgentPathsUpdate, AnalyticsSettingsUpdate,
    OnboardingCompletedSetParams, UserTimezoneUpdate, WorkspaceOnboardingFlagParams,
    WorkspaceOnboardingFlagSetParams, DEFAULT_ONBOARDING_FLAG_NAME,
};

pub(super) fn handle_workspace_onboarding_flag_exists(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkspaceOnboardingFlagParams>(params)?;
        to_json(
            config_rpc::workspace_onboarding_flag_resolve(
                &ctx,
                payload.flag_name,
                DEFAULT_ONBOARDING_FLAG_NAME,
            )
            .await?,
        )
    })
}

pub(super) fn handle_workspace_onboarding_flag_set(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkspaceOnboardingFlagSetParams>(params)?;
        to_json(
            config_rpc::workspace_onboarding_flag_set(
                &ctx,
                payload.flag_name,
                DEFAULT_ONBOARDING_FLAG_NAME,
                payload.value,
            )
            .await?,
        )
    })
}

pub(super) fn handle_update_analytics_settings(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<AnalyticsSettingsUpdate>(params)?;
        let patch = config_rpc::AnalyticsSettingsPatch {
            enabled: update.enabled,
        };
        to_json(config_rpc::load_and_apply_analytics_settings(&ctx, patch).await?)
    })
}

pub(super) fn handle_update_user_timezone(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<UserTimezoneUpdate>(params)?;
        to_json(config_rpc::load_and_apply_user_timezone(&ctx, update.timezone).await?)
    })
}

pub(super) fn handle_get_user_timezone(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(crate::core::Outcome::single_log(
            config_rpc::user_timezone_json(&config),
            "user time zone read",
        ))
    })
}

pub(super) fn handle_get_analytics_settings(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        use crate::core::Outcome;
        let config = ctx.load_config().await?;
        let result = serde_json::json!({
            "enabled": config.observability.analytics_enabled,
        });
        to_json(Outcome::new(
            result,
            vec!["analytics settings read".to_string()],
        ))
    })
}

pub(super) fn handle_get_dashboard_settings(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move { to_json(config_rpc::get_dashboard_settings(&ctx).await?) })
}

pub(super) fn handle_agent_server_status(
    _ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::agent_server_status()) })
}

pub(super) fn handle_reset_local_data(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move { to_json(config_rpc::reset_local_data(&ctx).await?) })
}

pub(crate) fn handle_get_data_paths(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] get_data_paths enter");
        match resolve_data_paths(&ctx, params).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] get_data_paths ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] get_data_paths fail: {err}");
                Err(err)
            }
        }
    })
}

/// Resolve the data paths for `get_data_paths`, honoring an optional `user_id`
/// param. The Clear App Data flow passes the signed-in id (#4950) because it
/// removes the active-user marker *before* the reset resolves paths — without
/// the explicit id the core would fall back to the pre-login `users/local` dir
/// and leave the real user's data behind. Absent/blank → marker-based
/// resolution (the default used by the agent tool and diagnostics).
async fn resolve_data_paths(
    ctx: &DispatchCtx,
    params: Map<String, Value>,
) -> Result<crate::core::Outcome<Value>, String> {
    let user_id = params
        .get("user_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| id.to_string());
    log::debug!(
        "[config][rpc] get_data_paths: explicit_user_id={}",
        user_id.is_some()
    );
    match user_id.as_deref() {
        Some(id) => config_rpc::get_data_paths_for_user(id).await,
        None => config_rpc::get_data_paths(ctx).await,
    }
}

pub(crate) fn handle_get_agent_paths(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] get_agent_paths enter");
        match config_rpc::get_agent_paths(&ctx).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] get_agent_paths ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] get_agent_paths fail: {err}");
                Err(err)
            }
        }
    })
}

pub(super) fn handle_update_agent_paths(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] update_agent_paths enter");
        let update = match deserialize_params::<AgentPathsUpdate>(params) {
            Ok(u) => u,
            Err(err) => {
                log::warn!("[config][rpc] update_agent_paths invalid params: {err}");
                return Err(err);
            }
        };
        let patch = config_rpc::AgentPathsPatch {
            action_dir: update.action_dir,
            files_dir: update.files_dir,
        };
        match config_rpc::load_and_apply_agent_paths_settings(&ctx, patch).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] update_agent_paths ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] update_agent_paths failed: {err}");
                Err(err)
            }
        }
    })
}

pub(super) fn handle_get_onboarding_completed(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move { to_json(config_rpc::get_onboarding_completed(&ctx).await?) })
}

pub(super) fn handle_set_onboarding_completed(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<OnboardingCompletedSetParams>(params)?;
        to_json(config_rpc::set_onboarding_completed(&ctx, payload.value).await?)
    })
}
