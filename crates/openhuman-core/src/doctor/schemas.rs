use crate::core::runtime::DispatchCtx;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::config::Config;
use crate::core::all::{
    BoundRegisteredController, ControllerEntry, ControllerFuture, ControllerTask,
    RegisteredController,
};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("report"), schemas("models")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("report"),
            handler: handle_report,
        },
        RegisteredController {
            schema: schemas("models"),
            handler: handle_models,
        },
    ]
}

pub(crate) fn all_controller_entries() -> Vec<ControllerEntry> {
    vec![
        ControllerEntry::bound(report_controller()),
        ControllerEntry::bound(models_controller()),
    ]
}

#[cfg(test)]
pub(crate) fn doctor_parameter_contracts() -> Vec<crate::core::all::ControllerParamContract> {
    tests::doctor_parameter_contracts()
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "report" => ControllerSchema {
            namespace: "doctor",
            function: "report",
            description: "Run diagnostics for workspace and runtime configuration.",
            inputs: vec![
                FieldSchema {
                    name: "tier",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Enum {
                        variants: vec!["static", "connect", "live"],
                    })),
                    comment: "Maximum doctor check tier to run.",
                    required: false,
                },
                FieldSchema {
                    name: "only",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Exact check ids to include.",
                    required: false,
                },
                FieldSchema {
                    name: "skip",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Exact check ids to exclude.",
                    required: false,
                },
                FieldSchema {
                    name: "profile",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Profile id, resolved by the host when supported.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "report",
                ty: TypeSchema::Ref("DoctorReport"),
                comment: "Aggregated diagnostics report.",
                required: true,
            }],
        },
        "models" => ControllerSchema {
            namespace: "doctor",
            function: "models",
            description: "Read-only inference provider catalog inspection.",
            inputs: vec![FieldSchema {
                name: "use_cache",
                ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                comment: "Reuse a recent catalog result when available.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "report",
                ty: TypeSchema::Ref("ModelProbeReport"),
                comment: "Model probe summary grouped by provider.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "doctor",
            function: "unknown",
            description: "Unknown doctor controller function.",
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

fn handle_report(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    report_controller().invoke(ctx, params)
}

fn handle_models(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    models_controller().invoke(ctx, params)
}

fn report_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(schemas("report"), parse_report, run_report)
}

fn models_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(
        schemas("models"),
        config_for_dispatch,
        parse_models,
        run_models,
    )
}

fn config_for_dispatch(ctx: DispatchCtx) -> ControllerTask<Config> {
    Box::pin(async move { crate::doctor::ops::config_for_dispatch(&ctx).await })
}

fn parse_report(params: Map<String, Value>) -> Result<crate::doctor::DoctorOptions, String> {
    Ok(crate::doctor::DoctorOptions {
        tier: read_optional::<crate::doctor::DoctorTier>(&params, "tier")?
            .unwrap_or(crate::doctor::DoctorTier::Connect),
        only: read_optional::<Vec<String>>(&params, "only")?.unwrap_or_default(),
        skip: read_optional::<Vec<String>>(&params, "skip")?.unwrap_or_default(),
        profile: read_optional::<String>(&params, "profile")?,
    })
}

fn parse_models(params: Map<String, Value>) -> Result<bool, String> {
    Ok(read_optional::<bool>(&params, "use_cache")?.unwrap_or(true))
}

fn run_report(ctx: DispatchCtx, options: crate::doctor::DoctorOptions) -> ControllerFuture {
    Box::pin(async move {
        to_json(crate::doctor::ops::doctor_report_for_dispatch(&ctx, options).await?)
    })
}

fn run_models(_ctx: DispatchCtx, config: Config, use_cache: bool) -> ControllerFuture {
    Box::pin(async move { to_json(crate::doctor::ops::doctor_models(&config, use_cache).await?) })
}

fn read_optional<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<Option<T>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|e| format!("invalid '{key}': {e}")),
    }
}

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
