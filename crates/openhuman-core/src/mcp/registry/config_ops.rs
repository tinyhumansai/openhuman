//! The `mcp.json` RPC handlers: reading the install store as one document and
//! replacing it from one. TinyMCP owns document validation and reconciliation;
//! this adapter publishes product events and schedules connections after the
//! module returns.

use serde_json::{json, Value};

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::core::Outcome;

// ── mcp.json ─────────────────────────────────────────────────────────────────

/// Renders the install store as the `mcp.json` document.
///
/// Credential *names* ride along as `envKeys`; values never do.
pub async fn mcp_clients_config_get(config: &Config) -> Result<Outcome<Value>, String> {
    let client = super::super::module_client::McpConfiguredClient::for_config(config)
        .map_err(|error| error.to_string())?;
    let doc: Value = client
        .call(
            &super::super::host::client_config(config).await,
            tinymcp_bus::names::methods::CONFIG_DOCUMENT_GET,
            (),
        )
        .await
        .map_err(|error| error.to_string())?;
    let count = doc["mcpServers"]
        .as_object()
        .map_or(0, serde_json::Map::len);

    Ok(Outcome::new(
        doc,
        vec![format!("config_get rendered {count} servers")],
    ))
}

/// Replaces the install store with what `doc` declares.
///
/// A replace, not a merge — TinyMCP applies the document and returns the
/// re-rendered result plus a change report. Enabled additions and updates
/// connect in the background; the status poll reports how that went.
pub async fn mcp_clients_config_set(config: &Config, doc: Value) -> Result<Outcome<Value>, String> {
    let client = super::super::module_client::McpConfiguredClient::for_config(config)
        .map_err(|error| error.to_string())?;
    let applied: tinymcp_bus::ConfigApplyResponse = client
        .call_confidential(
            &super::super::host::client_config(config).await,
            tinymcp_bus::names::methods::CONFIG_DOCUMENT_APPLY,
            (doc,),
        )
        .await
        .map_err(|error| error.to_string())?;
    let report = applied.report;

    for server in &report.removed {
        BUS.publish(DomainEvent::McpServerDisconnected {
            server_id: server.server_id.clone(),
            reason: Some("removed from mcp.json".to_string()),
        });
    }
    for server in &report.installed {
        BUS.publish(DomainEvent::McpServerInstalled {
            server_id: server.server_id.clone(),
            qualified_name: server.name.clone(),
        });
    }

    // Connecting can take the transport's whole timeout per server; the editor
    // must not wait on it. The status poll shows each attempt's outcome, and a
    // failure is recorded against the server either way.
    for server_id in report.connect_queued.clone() {
        let client = std::sync::Arc::clone(&client);
        let config = config.clone();
        tokio::spawn(async move {
            let outcome: Result<tinymcp_bus::ConnectOutcome, _> = client
                .call(
                    &super::super::host::client_config(&config).await,
                    tinymcp_bus::names::methods::CONNECT,
                    (server_id.clone(),),
                )
                .await;
            match outcome {
                Ok(outcome) => {
                    let tool_count = u32::try_from(outcome.tools.len()).unwrap_or(u32::MAX);
                    tracing::debug!(
                        server_id = %server_id,
                        tools = tool_count,
                        "[mcp] config_set connected a server"
                    );
                    BUS.publish(DomainEvent::McpServerConnected {
                        server_id,
                        tool_count,
                    });
                }
                Err(error) => {
                    tracing::warn!(
                        server_id = %server_id,
                        "[mcp] config_set could not connect a server: {error}"
                    );
                }
            }
        });
    }

    let mut rendered = applied.document;

    let names = |servers: &[tinymcp_bus::AppliedServer]| -> Vec<String> {
        servers.iter().map(|server| server.name.clone()).collect()
    };
    let note = format!(
        "config_set added={} updated={} removed={}",
        report.installed.len(),
        report.updated.len(),
        report.removed.len()
    );
    tracing::debug!(
        added = report.installed.len(),
        updated = report.updated.len(),
        removed = report.removed.len(),
        connecting = report.connect_queued.len(),
        "[mcp] config_set applied mcp.json"
    );
    if let Some(object) = rendered.as_object_mut() {
        object.insert("added".into(), json!(names(&report.installed)));
        object.insert("updated".into(), json!(names(&report.updated)));
        object.insert("removed".into(), json!(names(&report.removed)));
    }

    Ok(Outcome::new(rendered, vec![note]))
}

#[cfg(test)]
#[path = "config_ops_tests.rs"]
mod tests;
