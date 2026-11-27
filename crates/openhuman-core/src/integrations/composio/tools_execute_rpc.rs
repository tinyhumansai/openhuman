//! The `tools.composio_execute` controller: schema, registration and handler.
//!
//! The Tauri host's onboarding flow orchestrates Composio calls itself and
//! reaches them through this one `tools`-namespace controller. The wire name
//! stays `tools.composio_execute`; the `composio.*` namespace lives in
//! [`super::schemas`].

use crate::core::runtime::DispatchCtx;
use serde_json::{json, Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, Outcome, TypeSchema};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("tools_composio_execute")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("tools_composio_execute"),
        handler: handle_composio_execute,
    }]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "tools_composio_execute" => ControllerSchema {
            namespace: "tools",
            function: "composio_execute",
            description: "Execute a Composio action. Routes through the mode-aware \
                          factory: backend mode proxies via the OpenHuman backend; \
                          direct mode calls backend.composio.dev with the user's own \
                          API key. Exposed for Tauri-driven flows (e.g. onboarding) \
                          that orchestrate tool calls themselves.",
            inputs: vec![
                FieldSchema {
                    name: "action",
                    ty: TypeSchema::String,
                    comment: "Composio action slug (e.g. `GMAIL_FETCH_EMAILS`).",
                    required: true,
                },
                FieldSchema {
                    name: "params",
                    ty: TypeSchema::Json,
                    comment: "Action parameters object passed straight through to Composio.",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "successful",
                    ty: TypeSchema::Bool,
                    comment: "Whether the upstream provider reported success.",
                    required: true,
                },
                FieldSchema {
                    name: "data",
                    ty: TypeSchema::Json,
                    comment: "Raw provider response.",
                    required: true,
                },
                FieldSchema {
                    name: "error",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Provider error message if `successful` is false.",
                    required: false,
                },
            ],
        },
        _ => ControllerSchema {
            namespace: "tools",
            function: "unknown",
            description: "Unknown tools controller.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_composio_execute(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let action = params
            .get("action")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "missing required `action`".to_string())?;
        let action_args = params.get("params").cloned();

        let config = ctx.load_config().await?;
        let keys = crate::integrations::composio::tools::composio_call_keys(&config).await?;
        // The connector module owns the mode split (backend proxy vs. the
        // user's personal direct tenant, #1710); this only supplies the config.
        let resp = crate::integrations::composio::execute_dispatch::execute_composio_action(
            &config,
            // An RPC call is not a chat turn: no chat to route the disclosure to.
            None,
            &action,
            action_args,
            None,
        )
        .await
        .map_err(|e| format!("composio execute failed: {e}"))?;
        // Strip the Composio keys before the result leaves over RPC, as the
        // agent tools do with their results.
        let resp =
            crate::integrations::composio::tools::redact_composio_response(&config, keys, resp)
                .await?;
        tracing::debug!(
            action = %action,
            successful = resp.successful,
            "[tools][composio_execute] complete"
        );

        let payload = json!({
            "successful": resp.successful,
            "data": resp.data,
            "error": resp.error,
            "cost_usd": resp.cost_usd,
            "markdown_formatted": resp.markdown_formatted,
        });
        let log = vec![format!(
            "tools.composio_execute: action={action} successful={}",
            resp.successful
        )];
        Outcome::new(payload, log).into_cli_compatible_json()
    })
}

#[cfg(test)]
#[path = "tools_execute_rpc_tests.rs"]
mod tests;
