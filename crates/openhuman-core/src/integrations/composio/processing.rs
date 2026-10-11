//! Thin adapters for connector-owned argument and response processing.

use crate::config::Config;
use tinyconnectors_bus::{FilterResponseRequest, PrepareArgumentsRequest, PreparedArguments};

use super::module_client::methods;

/// Apply the connector's validation, calendar defaults and task window rules.
pub(crate) async fn prepare(
    config: &Config,
    tool: &str,
    arguments: Option<serde_json::Value>,
    timezone: Option<String>,
    since: Option<String>,
) -> Result<Option<serde_json::Value>, String> {
    prepare_with(
        &crate::modules::client::ModuleClient::new(config.clone()),
        tool,
        arguments,
        timezone,
        since,
    )
    .await
}

async fn prepare_with(
    client: &crate::modules::client::ModuleClient,
    tool: &str,
    arguments: Option<serde_json::Value>,
    timezone: Option<String>,
    since: Option<String>,
) -> Result<Option<serde_json::Value>, String> {
    let reply: PreparedArguments = client
        .call(
            "tinyconnectors",
            methods::PREPARE_ARGUMENTS,
            (PrepareArgumentsRequest {
                tool: tool.to_owned(),
                arguments,
                timezone,
                since,
            },),
        )
        .await
        .map_err(|error| error.to_string())?;
    match reply.error {
        Some(error) => Err(error.message),
        None => Ok(reply.arguments),
    }
}

/// Filter a provider response within the separately compiled module.
pub(crate) async fn filter(
    config: &Config,
    tool: &str,
    response: super::types::ComposioExecuteResponse,
    since: String,
) -> Result<super::types::ComposioExecuteResponse, String> {
    crate::modules::client::ModuleClient::new(config.clone())
        .call(
            "tinyconnectors",
            methods::FILTER_RESPONSE,
            (FilterResponseRequest {
                tool: tool.to_owned(),
                response,
                since,
            },),
        )
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "processing_tests.rs"]
mod tests;
