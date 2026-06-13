//! GitHub repo listing and trigger management ops.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::super::types::{
    ComposioActiveTriggersResponse, ComposioAvailableTriggersResponse,
    ComposioCreateTriggerResponse, ComposioDisableTriggerResponse, ComposioEnableTriggerResponse,
    ComposioGithubReposResponse, ComposioTriggerHistoryResult,
};
use super::error_utils::{report_composio_op_error, resolve_client, OpResult};

pub async fn composio_list_github_repos(
    config: &Config,
    connection_id: Option<String>,
) -> OpResult<RpcOutcome<ComposioGithubReposResponse>> {
    tracing::debug!(?connection_id, "[composio] rpc list_github_repos");
    let client = resolve_client(config)?;
    let resp = client
        .list_github_repos(connection_id.as_deref())
        .await
        .map_err(|e| {
            report_composio_op_error("list_github_repos", &e);
            format!("[composio] list_github_repos failed: {e:#}")
        })?;
    let count = resp.repositories.len();
    let connection_id = resp.connection_id.clone();
    Ok(RpcOutcome::new(
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
) -> OpResult<RpcOutcome<ComposioCreateTriggerResponse>> {
    tracing::debug!(slug = %slug, ?connection_id, "[composio] rpc create_trigger");
    let client = resolve_client(config)?;
    let resp = client
        .create_trigger(slug, connection_id.as_deref(), trigger_config)
        .await
        .map_err(|e| {
            report_composio_op_error("create_trigger", &e);
            format!("[composio] create_trigger failed: {e:#}")
        })?;
    let trigger_id = resp.trigger_id.clone();
    Ok(RpcOutcome::new(
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
) -> OpResult<RpcOutcome<ComposioAvailableTriggersResponse>> {
    tracing::debug!(toolkit = %toolkit, ?connection_id, "[composio] rpc list_available_triggers");
    let client = resolve_client(config)?;
    let resp = client
        .list_available_triggers(toolkit, connection_id.as_deref())
        .await
        .map_err(|e| {
            report_composio_op_error("list_available_triggers", &e);
            format!("[composio] list_available_triggers failed: {e:#}")
        })?;
    let count = resp.triggers.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!(
            "composio: {count} available trigger(s) for toolkit {toolkit}"
        )],
    ))
}

pub async fn composio_list_triggers(
    config: &Config,
    toolkit: Option<String>,
) -> OpResult<RpcOutcome<ComposioActiveTriggersResponse>> {
    tracing::debug!(?toolkit, "[composio] rpc list_triggers");
    let client = resolve_client(config)?;
    let resp = client
        .list_active_triggers(toolkit.as_deref())
        .await
        .map_err(|e| {
            report_composio_op_error("list_triggers", &e);
            format!("[composio] list_triggers failed: {e:#}")
        })?;
    let count = resp.triggers.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: {count} active trigger(s) listed")],
    ))
}

pub async fn composio_enable_trigger(
    config: &Config,
    connection_id: &str,
    slug: &str,
    trigger_config: Option<serde_json::Value>,
) -> OpResult<RpcOutcome<ComposioEnableTriggerResponse>> {
    tracing::debug!(slug = %slug, connection_id = %connection_id, "[composio] rpc enable_trigger");
    let client = resolve_client(config)?;
    let resp = client
        .enable_trigger(connection_id, slug, trigger_config)
        .await
        .map_err(|e| {
            report_composio_op_error("enable_trigger", &e);
            let raw = format!("{e:#}");
            let class = super::super::error_mapping::classify_composio_error(slug, &raw);
            let mapped = super::super::error_mapping::format_provider_error(slug, &raw);
            tracing::warn!(
                slug = %slug,
                connection_id = %connection_id,
                class = class.as_str(),
                "[composio] enable_trigger failed; surfacing mapped error"
            );
            mapped
        })?;
    let trigger_id = resp.trigger_id.clone();
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: enabled trigger {slug} → {trigger_id}")],
    ))
}

pub async fn composio_disable_trigger(
    config: &Config,
    trigger_id: &str,
) -> OpResult<RpcOutcome<ComposioDisableTriggerResponse>> {
    tracing::debug!(trigger_id = %trigger_id, "[composio] rpc disable_trigger");
    let client = resolve_client(config)?;
    let resp = client.disable_trigger(trigger_id).await.map_err(|e| {
        report_composio_op_error("disable_trigger", &e);
        format!("[composio] disable_trigger failed: {e:#}")
    })?;
    let message = if resp.deleted {
        format!("composio: disabled trigger {trigger_id}")
    } else {
        format!("composio: trigger {trigger_id} was not active")
    };
    Ok(RpcOutcome::new(resp, vec![message]))
}

pub async fn composio_list_trigger_history(
    config: &Config,
    limit: Option<usize>,
) -> OpResult<RpcOutcome<ComposioTriggerHistoryResult>> {
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
        "[composio] trigger history unavailable: archive store is not initialized".to_string()
    })?;

    let history = store
        .list_recent(requested_limit)
        .map_err(|error| format!("[composio] list_trigger_history failed: {error}"))?;
    let count = history.entries.len();

    Ok(RpcOutcome::new(
        history,
        vec![format!(
            "composio: {count} trigger history entrie(s) loaded (archive present)"
        )],
    ))
}
