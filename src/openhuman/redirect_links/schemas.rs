use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::redirect_links::ops as rl_ops;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("shorten"),
        schemas("expand"),
        schemas("list"),
        schemas("remove"),
        schemas("rewrite_inbound"),
        schemas("rewrite_outbound"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("shorten"),
            handler: handle_shorten,
        },
        RegisteredController {
            schema: schemas("expand"),
            handler: handle_expand,
        },
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("remove"),
            handler: handle_remove,
        },
        RegisteredController {
            schema: schemas("rewrite_inbound"),
            handler: handle_rewrite_inbound,
        },
        RegisteredController {
            schema: schemas("rewrite_outbound"),
            handler: handle_rewrite_outbound,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "shorten" => ControllerSchema {
            namespace: "redirect_links",
            function: "shorten",
            description: "Persist a long URL and return its `openhuman://link/<id>` short form.",
            inputs: vec![FieldSchema {
                name: "url",
                ty: TypeSchema::String,
                comment: "The full URL to shorten.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "link",
                ty: TypeSchema::Ref("RedirectLink"),
                comment: "The stored redirect link record.",
                required: true,
            }],
        },
        "expand" => ControllerSchema {
            namespace: "redirect_links",
            function: "expand",
            description: "Resolve a short id back to its full URL and bump hit count.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "The short id (the hex portion after `openhuman://link/`).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "link",
                ty: TypeSchema::Ref("RedirectLink"),
                comment: "The resolved redirect link record.",
                required: true,
            }],
        },
        "list" => ControllerSchema {
            namespace: "redirect_links",
            function: "list",
            description: "List stored redirect links, newest first.",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Maximum number of links to return (default 50, max 1000).",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "links",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("RedirectLink"))),
                comment: "Stored redirect links.",
                required: true,
            }],
        },
        "remove" => ControllerSchema {
            namespace: "redirect_links",
            function: "remove",
            description: "Delete a redirect link by id.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Redirect link id to remove.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "id",
                            ty: TypeSchema::String,
                            comment: "Id requested for removal.",
                            required: true,
                        },
                        FieldSchema {
                            name: "removed",
                            ty: TypeSchema::Bool,
                            comment: "True when a row was deleted.",
                            required: true,
                        },
                    ],
                },
                comment: "Removal result.",
                required: true,
            }],
        },
        "rewrite_inbound" => ControllerSchema {
            namespace: "redirect_links",
            function: "rewrite_inbound",
            description:
                "Rewrite every long URL in `text` to an `openhuman://link/<id>` placeholder \
                 to save tokens before a prompt hits the model. URLs shorter than `min_len` \
                 are left untouched.",
            inputs: vec![
                FieldSchema {
                    name: "text",
                    ty: TypeSchema::String,
                    comment: "Text to rewrite.",
                    required: true,
                },
                FieldSchema {
                    name: "min_len",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Minimum URL length to shorten; defaults to 80.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Ref("RewriteResult"),
                comment: "Rewritten text and per-URL replacement records.",
                required: true,
            }],
        },
        "rewrite_outbound" => ControllerSchema {
            namespace: "redirect_links",
            function: "rewrite_outbound",
            description:
                "Expand every `openhuman://link/<id>` placeholder in `text` back to its full \
                 URL before the message reaches the user.",
            inputs: vec![FieldSchema {
                name: "text",
                ty: TypeSchema::String,
                comment: "Text to rewrite.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Ref("RewriteResult"),
                comment: "Rewritten text and per-placeholder expansion records.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "redirect_links",
            function: "unknown",
            description: "Unknown redirect_links controller function.",
            inputs: vec![FieldSchema {
                name: "function",
                ty: TypeSchema::String,
                comment: "Unknown function requested for schema lookup.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_shorten(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let url = read_required::<String>(&params, "url")?;
        to_json(rl_ops::rl_shorten(&config, url.trim()).await?)
    })
}

fn handle_expand(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(rl_ops::rl_expand(&config, id.trim()).await?)
    })
}

fn handle_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let limit = read_optional_u64(&params, "limit")?
            .map(|raw| usize::try_from(raw).map_err(|_| "limit is too large for usize".to_string()))
            .transpose()?;
        to_json(rl_ops::rl_list(&config, limit).await?)
    })
}

fn handle_remove(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(rl_ops::rl_remove(&config, id.trim()).await?)
    })
}

fn handle_rewrite_inbound(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let text = read_required::<String>(&params, "text")?;
        let min_len = read_optional_u64(&params, "min_len")?
            .map(|raw| usize::try_from(raw).map_err(|_| "min_len too large for usize".to_string()))
            .transpose()?;
        to_json(rl_ops::rl_rewrite_inbound(&config, &text, min_len).await?)
    })
}

fn handle_rewrite_outbound(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let text = read_required::<String>(&params, "text")?;
        to_json(rl_ops::rl_rewrite_outbound(&config, &text).await?)
    })
}

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn read_optional_u64(params: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match params.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("invalid '{key}': expected unsigned integer")),
        Some(_) => Err(format!("invalid '{key}': expected unsigned integer")),
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_and_controllers_cover_every_function() {
        let names: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        assert_eq!(
            names,
            vec![
                "shorten",
                "expand",
                "list",
                "remove",
                "rewrite_inbound",
                "rewrite_outbound",
            ],
        );
        assert_eq!(all_registered_controllers().len(), 6);
    }

    #[test]
    fn schemas_unknown_returns_placeholder() {
        let s = schemas("does-not-exist");
        assert_eq!(s.function, "unknown");
    }

    #[test]
    fn shorten_schema_requires_url() {
        let s = schemas("shorten");
        assert_eq!(s.inputs.len(), 1);
        assert!(s.inputs[0].required);
    }
}
