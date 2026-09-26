//! Handlers for third-party integration settings: web search and Composio triggers.

use serde_json::{Map, Value};

use crate::config::rpc as config_rpc;
use crate::core::all::ControllerFuture;

use super::super::helpers::{
    deserialize_params, to_json, ComposioTriggerSettingsUpdate, SearchSettingsUpdate,
};

pub(super) fn handle_update_search_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] update_search_settings enter");
        let update = match deserialize_params::<SearchSettingsUpdate>(params) {
            Ok(u) => u,
            Err(err) => {
                log::warn!("[config][rpc] update_search_settings invalid params: {err}");
                return Err(err);
            }
        };
        let patch = config_rpc::SearchSettingsPatch {
            enabled: update.enabled,
            enabled_providers: update.enabled_providers,
            presentation: update.presentation,
            presentation_provider: update.presentation_provider,
            parallel_route: update.parallel_route,
            gemini_route: update.gemini_route,
            gemini_api_key: update.gemini_api_key,
            engine: update.engine,
            max_results: update.max_results,
            timeout_secs: update.timeout_secs,
            parallel_api_key: update.parallel_api_key,
            brave_api_key: update.brave_api_key,
            querit_api_key: update.querit_api_key,
            exa_api_key: update.exa_api_key,
            tavily_api_key: update.tavily_api_key,
            allowed_domains: update.allowed_domains,
            allow_all: update.allow_all,
        };
        match config_rpc::load_and_apply_search_settings(patch).await {
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

pub(super) fn handle_get_search_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        log::debug!("[config][rpc] get_search_settings enter");
        match config_rpc::get_search_settings().await {
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
        match config_rpc::load_and_apply_composio_trigger_settings(patch).await {
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
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async {
        log::debug!("[config][rpc] get_composio_trigger_settings enter");
        match config_rpc::get_composio_trigger_settings().await {
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
