//! The `modules` RPC namespace.
//!
//! Read-only plus one deliberate action. `list` and `status` report what this
//! build knows and what it has loaded; `load` forces a lazy module to resolve
//! now, which is what a settings screen offering "install this now" needs.
//!
//! There is no `unload`, and there cannot be: tinybus never unloads a library.
//! There is also no way to name an artifact over RPC — the loadable set is
//! compiled into [`super::registry`], and a method that could point the loader at
//! an arbitrary path would turn this namespace into remote code execution.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};
use std::sync::Arc;
use tinycomputer_bus::browser::SessionOptions;

use super::ops;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("status"),
        schemas("load"),
        schemas("browser_check_readiness"),
        schemas("browser_forget_sites"),
        schemas("computer_status"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("load"),
            handler: handle_load,
        },
        RegisteredController {
            schema: schemas("browser_check_readiness"),
            handler: handle_browser_check_readiness,
        },
        RegisteredController {
            schema: schemas("browser_forget_sites"),
            handler: handle_browser_forget_sites,
        },
        RegisteredController {
            schema: schemas("computer_status"),
            handler: handle_computer_status,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "modules",
            function: "list",
            description: "List every loadable module this build knows, with its state.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "modules",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("ModuleStatus"))),
                comment: "Status of each known module.",
                required: true,
            }],
        },
        "status" => ControllerSchema {
            namespace: "modules",
            function: "status",
            description: "Report the state of one module by id.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Registry identifier, e.g. `tinydocs`.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "module",
                ty: TypeSchema::Ref("ModuleStatus"),
                comment: "Status of the requested module.",
                required: true,
            }],
        },
        "load" => ControllerSchema {
            namespace: "modules",
            function: "load",
            description: "Resolve and load a module now instead of on first use.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Registry identifier, e.g. `tinydocs`.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "module",
                ty: TypeSchema::Ref("ModuleStatus"),
                comment: "Status after the load attempt.",
                required: true,
            }],
        },
        "browser_check_readiness" => ControllerSchema {
            namespace: "modules",
            function: "browser_check_readiness",
            description: "Load TinyComputer and briefly launch Chrome to check browser readiness.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "module_ready",
                    ty: TypeSchema::Bool,
                    comment: "TinyComputer module is serving its browser members.",
                    required: true,
                },
                FieldSchema {
                    name: "chrome_ready",
                    ty: TypeSchema::Bool,
                    comment: "Chrome launched and closed successfully.",
                    required: true,
                },
                FieldSchema {
                    name: "error",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Sanitized setup failure, if any.",
                    required: false,
                },
            ],
        },
        "browser_forget_sites" => ControllerSchema {
            namespace: "modules",
            function: "browser_forget_sites",
            description: "Forget what finished browser tasks learned about one site, or about every site.",
            inputs: vec![FieldSchema {
                name: "site",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "The site's host or an address on it; omit to forget every site.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "forgotten",
                ty: TypeSchema::U64,
                comment: "How many sites were forgotten.",
                required: true,
            }],
        },
        "computer_status" => ControllerSchema {
            namespace: "modules",
            function: "computer_status",
            description: "Report the TinyComputer module, its decision and planner routes, and optionally what it says is configured.",
            inputs: vec![FieldSchema {
                name: "load",
                ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                comment: "Load the module and ask it (Describe) even when it is not serving yet.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Json,
                comment: "Module status, decision_model, decision_route, planner_route, and capabilities or error.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "modules",
            function: "unknown",
            description: "Unknown modules controller function.",
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

/// What the settings page's Chrome check shows when Chrome would not start:
/// where to fix a missing Chrome or a Chrome path that names no Chrome, or
/// else the module's own reason.
fn readiness_error(detail: &str) -> String {
    if super::browser::chrome_not_found(detail) {
        "Chrome was not found. Set Chrome path below to the Chrome program (for example \
         /Applications/Google Chrome.app/Contents/MacOS/Google Chrome), save, and test again."
            .to_owned()
    } else if super::browser::chrome_path_unusable(detail) {
        "Chrome could not be started from Chrome path. Set it to the Chrome program itself \
         (for example /Applications/Google Chrome.app/Contents/MacOS/Google Chrome), save, and \
         test again."
            .to_owned()
    } else {
        format!("Chrome could not start: {detail}")
    }
}

fn handle_browser_check_readiness(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let mut config = ctx.load_config().await?;
        crate::core::runtime::components::require_config(&config, "computer")?;
        // The probe never navigates. Give its disposable session a non-routable
        // origin so an empty website allowlist does not prevent a Chrome check.
        config.http_request.allowed_domains = vec!["example.invalid".into()];
        let client = super::browser::BrowserClient::new(Arc::new(config));
        if client.ensure_ready().await.is_err() {
            return Ok(
                serde_json::json!({"module_ready": false, "chrome_ready": false, "error": "TinyComputer module is unavailable; configure a local module override"}),
            );
        }
        let opened = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            client.open_session(SessionOptions::default()),
        )
        .await;
        match opened {
            Ok(Ok(session)) => {
                // A session is always closed after this non-navigating probe.
                let closed = client.close_session(&session.id).await.is_ok();
                Ok(
                    serde_json::json!({"module_ready": true, "chrome_ready": closed,
                    "error": if closed { None } else { Some("Chrome session could not close cleanly") }}),
                )
            }
            Ok(Err(error)) => Ok(
                serde_json::json!({"module_ready": true, "chrome_ready": false,
                "error": readiness_error(&error.to_string())}),
            ),
            Err(_) => Ok(
                serde_json::json!({"module_ready": true, "chrome_ready": false,
                "error": "Chrome launch timed out"}),
            ),
        }
    })
}

fn handle_browser_forget_sites(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        // A blank or non-text site must not read as "every site".
        let site = match params.get("site") {
            None | Some(Value::Null) => None,
            Some(Value::String(site)) if !site.trim().is_empty() => Some(site.trim().to_owned()),
            Some(_) => {
                return Err(
                    "`site` must name a site (a host or an address); omit it to forget every site"
                        .to_owned(),
                )
            }
        };
        let config = ctx.load_config().await?;
        crate::core::runtime::components::require_config(&config, "computer")?;
        let forgotten = super::site_memory_host::forget(&config, site.as_deref())
            .await
            .map_err(|error| format!("could not forget learned site data: {error}"))?;
        tracing::info!(
            forgotten,
            all = site.is_none(),
            "[browser-sites] forgot learned site data"
        );
        Ok(serde_json::json!({ "forgotten": forgotten }))
    })
}

fn handle_computer_status(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let load = params.get("load").and_then(Value::as_bool).unwrap_or(false);
        let config = ctx.load_config().await?;
        crate::core::runtime::components::require_config(&config, "computer")?;
        controller_value(super::computer::status(&config, load).await)
    })
}

fn controller_value(value: impl serde::Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

fn handle_list(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        Ok(serde_json::json!({ "modules": ops::list(&config) }))
    })
}

fn handle_status(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let id = string_param(&params, "id").ok_or("`id` is required")?;
        let config = ctx.load_config().await?;
        match ops::list(&config).into_iter().find(|m| m.id == id) {
            Some(module) => Ok(serde_json::json!({ "module": module })),
            None => Err(format!("unknown module '{id}'")),
        }
    })
}

fn handle_load(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let id = string_param(&params, "id").ok_or("`id` is required")?;
        let config = ctx.load_config().await?;
        // A failed load is reported through the returned status rather than as
        // an RPC error: the caller asked "what happened", and the answer is a
        // state plus a reason, not a transport failure.
        if let Err(reason) = ops::ensure_loaded(&config, &id).await {
            log::warn!("[modules] explicit load of '{id}' failed: {reason}");
        }
        match ops::list(&config).into_iter().find(|m| m.id == id) {
            Some(module) => Ok(serde_json::json!({ "module": module })),
            None => Err(format!("unknown module '{id}'")),
        }
    })
}

/// A required string parameter, rejecting a blank one.
fn string_param(params: &Map<String, Value>, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
