//! GitHub repo listing and trigger management ops.
//!
//! # Where the webhook half lives
//!
//! Creating and enabling a subscription is the module's, because it needs the
//! credential. *Receiving* a delivery is not: the backend HMAC-verifies each
//! webhook and fans it out over the user's socket, and the module has no
//! socket. `super::super::bus`'s subscriber keeps that job.
//!
//! The archive sits on the module's side of that line, because the module is
//! what writes to it as deliveries are dispatched.

use crate::config::Config;
use crate::core::Outcome;

use super::super::module_client::{self as connectors, methods};
use super::super::types::{
    ComposioActiveTriggersResponse, ComposioAvailableTriggersResponse,
    ComposioCreateTriggerRequest, ComposioCreateTriggerResponse, ComposioDisableTriggerRequest,
    ComposioDisableTriggerResponse, ComposioEnableTriggerRequest, ComposioEnableTriggerResponse,
    ComposioGithubReposResponse, ComposioListAvailableTriggersRequest,
    ComposioListGithubReposRequest, ComposioListTriggersRequest, ComposioTriggerHistoryResult,
};
use super::error_utils::{report_composio_op_error, OpResult};

pub async fn composio_list_github_repos(
    config: &Config,
    connection_id: Option<String>,
) -> OpResult<Outcome<ComposioGithubReposResponse>> {
    tracing::debug!(?connection_id, "[composio] rpc list_github_repos");
    let resp = connectors::call::<_, ComposioGithubReposResponse>(
        config,
        methods::LIST_GITHUB_REPOS,
        ComposioListGithubReposRequest { connection_id },
    )
    .await
    .map_err(|e| {
        report_composio_op_error("list_github_repos", &anyhow::anyhow!("{e}"));
        format!("[composio] list_github_repos failed: {e}")
    })?;
    let count = resp.repositories.len();
    let connection_id = resp.connection_id.clone();
    Ok(Outcome::new(
        resp,
        vec![format!(
            "composio: {count} github repo(s) listed for connection {connection_id}"
        )],
    ))
}

pub async fn composio_create_trigger(
    config: &Config,
    slug: &str,
    connection_id: Option<String>,
    trigger_config: Option<serde_json::Value>,
) -> OpResult<Outcome<ComposioCreateTriggerResponse>> {
    tracing::debug!(slug = %slug, ?connection_id, "[composio] rpc create_trigger");
    let resp = connectors::call::<_, ComposioCreateTriggerResponse>(
        config,
        methods::CREATE_TRIGGER,
        ComposioCreateTriggerRequest {
            slug: slug.to_string(),
            connection_id,
            trigger_config,
        },
    )
    .await
    .map_err(|e| {
        report_composio_op_error("create_trigger", &anyhow::anyhow!("{e}"));
        format!("[composio] create_trigger failed: {e}")
    })?;
    let trigger_id = resp.trigger_id.clone();
    Ok(Outcome::new(
        resp,
        vec![format!(
            "composio: trigger {trigger_id} created for slug {slug}"
        )],
    ))
}

pub async fn composio_list_available_triggers(
    config: &Config,
    toolkit: &str,
    connection_id: Option<String>,
) -> OpResult<Outcome<ComposioAvailableTriggersResponse>> {
    tracing::debug!(toolkit = %toolkit, ?connection_id, "[composio] rpc list_available_triggers");
    let resp = connectors::call::<_, ComposioAvailableTriggersResponse>(
        config,
        methods::LIST_AVAILABLE_TRIGGERS,
        ComposioListAvailableTriggersRequest {
            toolkit: toolkit.to_string(),
            connection_id,
        },
    )
    .await
    .map_err(|e| {
        report_composio_op_error("list_available_triggers", &anyhow::anyhow!("{e}"));
        format!("[composio] list_available_triggers failed: {e}")
    })?;
    let count = resp.triggers.len();
    Ok(Outcome::new(
        resp,
        vec![format!(
            "composio: {count} available trigger(s) for toolkit {toolkit}"
        )],
    ))
}

pub async fn composio_list_triggers(
    config: &Config,
    toolkit: Option<String>,
) -> OpResult<Outcome<ComposioActiveTriggersResponse>> {
    tracing::debug!(?toolkit, "[composio] rpc list_triggers");
    let resp = connectors::call::<_, ComposioActiveTriggersResponse>(
        config,
        methods::LIST_TRIGGERS,
        ComposioListTriggersRequest { toolkit },
    )
    .await
    .map_err(|e| {
        report_composio_op_error("list_triggers", &anyhow::anyhow!("{e}"));
        format!("[composio] list_triggers failed: {e}")
    })?;
    let count = resp.triggers.len();
    Ok(Outcome::new(
        resp,
        vec![format!("composio: {count} active trigger(s) listed")],
    ))
}

pub async fn composio_enable_trigger(
    config: &Config,
    connection_id: &str,
    slug: &str,
    trigger_config: Option<serde_json::Value>,
) -> OpResult<Outcome<ComposioEnableTriggerResponse>> {
    tracing::debug!(slug = %slug, connection_id = %connection_id, "[composio] rpc enable_trigger");
    let resp = connectors::call::<_, ComposioEnableTriggerResponse>(
        config,
        methods::ENABLE_TRIGGER,
        ComposioEnableTriggerRequest {
            connection_id: connection_id.to_string(),
            slug: slug.to_string(),
            trigger_config,
        },
    )
    .await;
    let resp = match resp {
        Ok(response) => response,
        Err(e) => {
            if e.contains("MODULE_CALL_REPORTED:") {
                return Err(e);
            }
            report_composio_op_error("enable_trigger", &anyhow::anyhow!("{e}"));
            // Enabling is the one trigger call a user drives directly from a
            // settings screen, so its failures are mapped to something a person can
            // act on ("reconnect GitHub") rather than the provider's own wording.
            let classified: tinyconnectors_bus::ProviderError =
                crate::modules::client::ModuleClient::new(config.clone())
                    .call(
                        "tinyconnectors",
                        methods::CLASSIFY_ERROR,
                        (tinyconnectors_bus::ClassifyErrorRequest {
                            tool: slug.to_owned(),
                            message: e,
                        },),
                    )
                    .await
                    .map_err(|error| error.to_string())?;
            tracing::warn!(
                slug = %slug,
                connection_id = %connection_id,
                class = classified.class.as_str(),
                "[composio] enable_trigger failed; surfacing mapped error"
            );
            return Err(classified.message);
        }
    };
    let trigger_id = resp.trigger_id.clone();
    Ok(Outcome::new(
        resp,
        vec![format!("composio: enabled trigger {slug} → {trigger_id}")],
    ))
}

pub async fn composio_disable_trigger(
    config: &Config,
    trigger_id: &str,
) -> OpResult<Outcome<ComposioDisableTriggerResponse>> {
    tracing::debug!(trigger_id = %trigger_id, "[composio] rpc disable_trigger");
    let resp = connectors::call::<_, ComposioDisableTriggerResponse>(
        config,
        methods::DISABLE_TRIGGER,
        ComposioDisableTriggerRequest {
            trigger_id: trigger_id.to_string(),
        },
    )
    .await
    .map_err(|e| {
        report_composio_op_error("disable_trigger", &anyhow::anyhow!("{e}"));
        format!("[composio] disable_trigger failed: {e}")
    })?;
    let message = if resp.deleted {
        format!("composio: disabled trigger {trigger_id}")
    } else {
        format!("composio: trigger {trigger_id} was not active")
    };
    Ok(Outcome::new(resp, vec![message]))
}

pub async fn composio_list_trigger_history(
    config: &Config,
    limit: Option<usize>,
) -> OpResult<Outcome<ComposioTriggerHistoryResult>> {
    let requested_limit = limit.unwrap_or(100).clamp(1, 500);
    let workspace_label = config
        .workspace_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("<workspace>");
    tracing::debug!(
        limit = requested_limit,
        workspace = workspace_label,
        "[composio] rpc list_trigger_history"
    );

    let store = super::super::trigger_history::global().ok_or_else(|| {
        "[composio] list_trigger_history failed: trigger archive owner unavailable".to_owned()
    })?;
    let history = store
        .read(config, Some(requested_limit))
        .await
        .map_err(|error| format!("[composio] list_trigger_history failed: {error}"))?;
    let count = history.entries.len();

    Ok(Outcome::new(
        history,
        vec![format!(
            "composio: {count} trigger history entrie(s) loaded (archive present)"
        )],
    ))
}
