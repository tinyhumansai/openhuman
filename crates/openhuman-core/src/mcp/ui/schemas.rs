//! `mcp_ui` controller schemas and handlers.

use std::sync::Arc;

use serde_json::{Map, Value};

use crate::config::rpc as config_rpc;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::security::SecurityPolicy;
use crate::util::read_optional;

use super::ops;
use super::port::HostUiPort;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("resource_read"), schemas("tool_call")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("resource_read"),
            handler: handle_resource_read,
        },
        RegisteredController {
            schema: schemas("tool_call"),
            handler: handle_tool_call,
        },
    ]
}

fn field(name: &'static str, ty: TypeSchema, comment: &'static str, required: bool) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required,
    }
}

fn optional(ty: TypeSchema) -> TypeSchema {
    TypeSchema::Option(Box::new(ty))
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "resource_read" => ControllerSchema {
            namespace: "mcp_ui",
            function: "resource_read",
            description: "Read the HTML document a tool's widget renders: a ui:// resource on a connected MCP server, or an inline document by id.",
            inputs: vec![
                field("server_id", optional(TypeSchema::String), "Server the widget belongs to.", false),
                field("uri", optional(TypeSchema::String), "The ui:// resource URI.", false),
                field("inline_id", optional(TypeSchema::String), "An inline document id from a tool result.", false),
            ],
            outputs: vec![
                field("html", TypeSchema::String, "The widget document.", true),
                field("mime_type", TypeSchema::String, "The document's MIME type.", true),
                field("csp", TypeSchema::Json, "Declared https origins: connect_domains, resource_domains.", true),
                field("permissions", optional(TypeSchema::Json), "Declared permissions.", false),
                field("prefers_border", TypeSchema::Bool, "Whether the widget asks for a border.", true),
            ],
        },
        "tool_call" => ControllerSchema {
            namespace: "mcp_ui",
            function: "tool_call",
            description: "Call a tool on behalf of a widget, under the agent's tool policy. Tools that are not read-only need confirmed=true.",
            inputs: vec![
                field("server_id", TypeSchema::String, "Server the widget belongs to.", true),
                field("tool_name", TypeSchema::String, "Tool to call.", true),
                field("arguments", optional(TypeSchema::Json), "Arguments object.", false),
                field("confirmed", optional(TypeSchema::Bool), "The user approved this call.", false),
            ],
            outputs: vec![
                field("requires_confirmation", TypeSchema::Bool, "True when the call waits for the user.", true),
                field("read_only", TypeSchema::Bool, "Whether the tool declares itself read-only.", true),
                field("result", optional(TypeSchema::Json), "The raw tools/call result.", false),
                field("is_error", optional(TypeSchema::Bool), "True when the tool reported an error.", false),
            ],
        },
        _ => ControllerSchema {
            namespace: "mcp_ui",
            function: "unknown",
            description: "Unknown mcp_ui controller function.",
            inputs: vec![],
            outputs: vec![field("error", TypeSchema::String, "Lookup error details.", true)],
        },
    }
}

fn handle_resource_read(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let server_id = read_optional::<String>(&params, "server_id")?;
        let uri = read_optional::<String>(&params, "uri")?;
        let inline_id = read_optional::<String>(&params, "inline_id")?;
        let config = config_rpc::load_config_with_timeout().await?;
        let port = HostUiPort::new(Arc::new(config));
        let outcome = ops::resource_read(&port, server_id, uri, inline_id).await?;
        Ok(outcome.value)
    })
}

fn handle_tool_call(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let server_id = read_optional::<String>(&params, "server_id")?;
        let tool_name = read_optional::<String>(&params, "tool_name")?;
        let arguments = params.get("arguments").cloned();
        let confirmed = read_optional::<bool>(&params, "confirmed")?.unwrap_or(false);
        let config = config_rpc::load_config_with_timeout().await?;
        let security = crate::security::live_policy::current().unwrap_or_else(|| {
            Arc::new(SecurityPolicy::from_config(
                &config.autonomy,
                &config.workspace_dir,
                &config.action_dir,
            ))
        });
        let port = HostUiPort::new(Arc::new(config));
        let outcome =
            ops::tool_call(&port, &security, server_id, tool_name, arguments, confirmed).await?;
        Ok(outcome.value)
    })
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
