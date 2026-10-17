//! Handlers for third-party integration settings: web search and Composio triggers.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::config::rpc as config_rpc;
use crate::core::all::ControllerFuture;

use super::super::helpers::{deserialize_params, to_json, ComposioTriggerSettingsUpdate};

pub(super) fn handle_update_search_settings(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] update_search_settings enter");
        let patch = match deserialize_params::<config_rpc::SearchSettingsPatch>(params) {
            Ok(patch) => patch,
            Err(err) => {
                log::warn!("[config][rpc] update_search_settings invalid params: {err}");
                return Err(err);
            }
        };
        match config_rpc::load_and_apply_search_settings(&ctx, patch).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] update_search_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] update_search_settings failed: {err}");
                Err(err)
            }
        }
    })
}

pub(super) fn handle_get_search_settings(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] get_search_settings enter");
        match config_rpc::get_search_settings(&ctx).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] get_search_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] get_search_settings failed: {err}");
                Err(err)
            }
        }
    })
}

pub(super) fn handle_update_composio_trigger_settings(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] update_composio_trigger_settings enter");
        let update = match deserialize_params::<ComposioTriggerSettingsUpdate>(params) {
            Ok(u) => u,
            Err(err) => {
                log::warn!("[config][rpc] update_composio_trigger_settings invalid params: {err}");
                return Err(err);
            }
        };
        let patch = config_rpc::ComposioTriggerSettingsPatch {
            triage_disabled: update.triage_disabled,
            triage_disabled_toolkits: update.triage_disabled_toolkits,
        };
        match config_rpc::load_and_apply_composio_trigger_settings(&ctx, patch).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] update_composio_trigger_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] update_composio_trigger_settings failed: {err}");
                Err(err)
            }
        }
    })
}

pub(super) fn handle_get_composio_trigger_settings(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] get_composio_trigger_settings enter");
        match config_rpc::get_composio_trigger_settings(&ctx).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] get_composio_trigger_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] get_composio_trigger_settings failed: {err}");
                Err(err)
            }
        }
    })
}
