//! Controller schemas + handlers for `http_host`.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::types::{HostedDirLookupParams, StartHostedDirParams};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("start"),
        schemas("stop"),
        schemas("get"),
        schemas("list"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("start"),
            handler: handle_start,
        },
        RegisteredController {
            schema: schemas("stop"),
            handler: handle_stop,
        },
        RegisteredController {
            schema: schemas("get"),
            handler: handle_get,
        },
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "start" => ControllerSchema {
            namespace: "http_host",
            function: "start",
            description: "Host a local directory over HTTP on the requested port. \
                Basic auth is enabled by default using the active username plus a generated password.",
            inputs: vec![
                FieldSchema {
                    name: "directory",
                    ty: TypeSchema::String,
                    comment: "Absolute or relative path to the directory to host.",
                    required: true,
                },
                FieldSchema {
                    name: "port",
                    ty: TypeSchema::U64,
                    comment: "TCP port to bind. Use 0 to let the OS choose a free port.",
                    required: true,
                },
                FieldSchema {
                    name: "bind_host",
                    ty: TypeSchema::String,
                    comment: "Interface / host to bind. Defaults to 127.0.0.1; use an explicit non-loopback host for LAN reachability.",
                    required: false,
                },
                FieldSchema {
                    name: "server_name",
                    ty: TypeSchema::String,
                    comment: "Optional caller-provided label for the hosted server.",
                    required: false,
                },
                FieldSchema {
                    name: "disable_auth",
                    ty: TypeSchema::Bool,
                    comment: "Disable basic auth. Defaults to false and should be used sparingly.",
                    required: false,
                },
                FieldSchema {
                    name: "username",
                    ty: TypeSchema::String,
                    comment: "Optional override for the basic-auth username when auth is enabled.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "server",
                ty: TypeSchema::Json,
                comment: "Hosted server details including URLs and auth credentials.",
                required: true,
            }],
        },
        "stop" => ControllerSchema {
            namespace: "http_host",
            function: "stop",
            description: "Stop a previously started hosted-directory HTTP server.",
            inputs: vec![FieldSchema {
                name: "server_id",
                ty: TypeSchema::String,
                comment: "Opaque server id returned by http_host.start.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "stopped",
                    ty: TypeSchema::Bool,
                    comment: "Whether the server was successfully stopped.",
                    required: true,
                },
                FieldSchema {
                    name: "server",
                    ty: TypeSchema::Json,
                    comment: "The stopped server's final configuration snapshot.",
                    required: true,
                },
            ],
        },
        "get" => ControllerSchema {
            namespace: "http_host",
            function: "get",
            description: "Fetch a hosted-directory HTTP server by id.",
            inputs: vec![FieldSchema {
                name: "server_id",
                ty: TypeSchema::String,
                comment: "Opaque server id returned by http_host.start.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "server",
                ty: TypeSchema::Json,
                comment: "Hosted server details including current auth credentials.",
                required: true,
            }],
        },
        "list" => ControllerSchema {
            namespace: "http_host",
            function: "list",
            description: "List all currently running hosted-directory HTTP servers.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "servers",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "All active hosted servers.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "http_host",
            function: "unknown",
            description: "Unknown http_host controller function.",
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

fn handle_start(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params: StartHostedDirParams = parse(params)?;
        crate::openhuman::http_host::rpc::start(params)
            .await?
            .into_cli_compatible_json()
    })
}

fn handle_stop(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params: HostedDirLookupParams = parse(params)?;
        crate::openhuman::http_host::rpc::stop(params)
            .await?
            .into_cli_compatible_json()
    })
}

fn handle_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params: HostedDirLookupParams = parse(params)?;
        crate::openhuman::http_host::rpc::get(params)
            .await?
            .into_cli_compatible_json()
    })
}

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        crate::openhuman::http_host::rpc::list()
            .await?
            .into_cli_compatible_json()
    })
}

fn parse<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_inventory_matches_handlers() {
        assert_eq!(
            all_controller_schemas().len(),
            all_registered_controllers().len()
        );
    }

    #[test]
    fn start_schema_requires_directory_and_port() {
        let schema = schemas("start");
        let required: Vec<&str> = schema
            .inputs
            .iter()
            .filter(|field| field.required)
            .map(|field| field.name)
            .collect();
        assert_eq!(required, vec!["directory", "port"]);
    }

    #[test]
    fn stop_schema_requires_server_id() {
        let schema = schemas("stop");
        assert_eq!(schema.inputs.len(), 1);
        assert_eq!(schema.inputs[0].name, "server_id");
        assert!(schema.inputs[0].required);
    }

    #[test]
    fn unknown_schema_falls_back() {
        let schema = schemas("nope");
        assert_eq!(schema.namespace, "http_host");
        assert_eq!(schema.function, "unknown");
    }
}
