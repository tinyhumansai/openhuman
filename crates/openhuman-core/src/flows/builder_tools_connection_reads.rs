//! Flow-owned composition provider for the mixed Composio/HTTP connection picker.

use async_trait::async_trait;
use serde_json::Value;

pub(crate) struct FlowsConnectionProvider;

#[async_trait]
impl crate::integrations::composio::builder_tools::ConnectableToolkitProvider
    for FlowsConnectionProvider
{
    async fn connected(
        &self,
        dispatch: &crate::core::runtime::DispatchCtx,
        config: &crate::config::Config,
    ) -> Vec<String> {
        crate::flows::ops::connected_toolkits(dispatch, config)
            .await
            .into_iter()
            .collect()
    }
}

#[async_trait]
impl crate::integrations::composio::builder_tools::FlowConnectionProvider
    for FlowsConnectionProvider
{
    async fn list(
        &self,
        dispatch: &crate::core::runtime::DispatchCtx,
        config: &crate::config::Config,
    ) -> Result<Vec<Value>, String> {
        let outcome = crate::flows::ops::flows_list_connections(dispatch, config).await?;
        Ok(outcome.value.iter().map(flow_connection_to_json).collect())
    }
}

/// Render a non-secret connection summary for the agent-facing picker.
pub(crate) fn flow_connection_to_json(c: &crate::flows::ops::FlowConnection) -> Value {
    serde_json::json!({
        "connection_ref": c.connection_ref,
        "kind": c.kind,
        "display": c.display,
        "toolkit": c.toolkit,
        "scheme": c.scheme,
        "platform_user_id": c.platform_user_id,
    })
}
