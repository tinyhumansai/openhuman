use super::*;
use crate::core::runtime::DispatchCtx;
use tinyflows_bus::{GraphValidation, ImportGraphResult, ValidateGraphRequest};

// The refusal code remains stable on OpenHuman's test surface.
#[cfg(test)]
#[cfg(test)]
pub(crate) use tinyflows::compat::UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN;

pub(super) async fn validate_over_bus(
    ctx: &DispatchCtx,
    config: &Config,
    graph: Value,
) -> Result<GraphValidation, String> {
    let registration =
        crate::flows::authoring_host::register(Arc::new(config.clone()), ctx.clone()).await?;
    let proxy = crate::modules::tinyflows::authoring_proxy(config).await?;
    proxy
        .call(
            tinyflows_bus::methods::VALIDATE_GRAPH,
            &(ValidateGraphRequest {
                graph,
                host: Some(registration.authoring.clone()),
            },),
        )
        .await
        .map_err(|error| error.wire_message())
}

/// Returns the owner's migrated graph for callers that need parsing/migration
/// without applying the host's save or execution policy.
pub(super) async fn migrate_over_bus(
    ctx: &DispatchCtx,
    config: &Config,
    graph: Value,
) -> Result<Value, String> {
    let report = validate_over_bus(ctx, config, graph).await?;
    report.graph.ok_or_else(|| {
        report
            .errors
            .first()
            .map(|error| error.message.clone())
            .unwrap_or_else(|| "TinyFlows validation returned no graph".to_string())
    })
}

fn is_engine_compatibility_error(code: &str) -> bool {
    matches!(
        code,
        "unsupported_nested_conditional_fan_in"
            | "unsupported_main_port_conditional_fan_in"
            | "incompatible_referenced_workflow"
    )
}

/// Validates and migrates a graph through TinyFlows, resolving saved children
/// through the host callback bound to the current invocation scope.
pub(crate) async fn validate_and_migrate_graph(
    ctx: &DispatchCtx,
    config: &Config,
    graph: Value,
) -> Result<Value, String> {
    let report = validate_over_bus(ctx, config, graph).await?;
    if let Some(error) = report
        .errors
        .iter()
        .find(|error| !is_engine_compatibility_error(&error.code))
    {
        return Err(error.message.clone());
    }
    report
        .graph
        .ok_or_else(|| "TinyFlows validation returned no graph".to_string())
}

/// Host-aware compatibility errors for graphs that can reference saved flows.
pub(crate) async fn config_aware_engine_compatibility_errors(
    ctx: &DispatchCtx,
    config: &Config,
    graph: Value,
) -> Result<Vec<String>, String> {
    let report = validate_over_bus(ctx, config, graph).await?;
    Ok(report
        .errors
        .into_iter()
        .filter(|error| is_engine_compatibility_error(&error.code))
        .map(|error| format!("{}: {}", error.code, error.message))
        .collect())
}

pub(super) async fn ensure_config_aware_engine_compatible(
    ctx: &DispatchCtx,
    config: &Config,
    graph: &Value,
) -> Result<(), String> {
    match config_aware_engine_compatibility_errors(ctx, config, graph.clone())
        .await?
        .into_iter()
        .next()
    {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Validates a candidate graph without persisting it and returns the owner
/// module's trigger warnings after successful graph validation.
pub async fn flows_validate(
    ctx: &DispatchCtx,
    config: &Config,
    graph_json: Value,
) -> Outcome<tinyflows_bus::copilot::FlowValidation> {
    use tinyflows_bus::copilot::{FlowValidation, FlowValidationError};
    let report = match validate_over_bus(ctx, config, graph_json).await {
        Ok(report) => report,
        Err(error) => {
            return Outcome::single_log(
                FlowValidation {
                    valid: false,
                    errors: vec![error.clone()],
                    error_details: vec![FlowValidationError {
                        code: "authoring_unavailable".into(),
                        message: error,
                        node_id: None,
                        field: None,
                    }],
                    warnings: Vec::new(),
                },
                "flow validation failed",
            );
        }
    };
    let errors = report
        .errors
        .into_iter()
        .map(|error| FlowValidationError {
            code: error.code,
            message: error.message,
            node_id: error.node_id,
            field: error.field,
        })
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        let messages = errors.iter().map(|error| error.message.clone()).collect();
        return Outcome::single_log(
            FlowValidation {
                valid: false,
                errors: messages,
                error_details: errors,
                warnings: Vec::new(),
            },
            "flow validation failed",
        );
    }
    if report.graph.is_none() {
        let error = "TinyFlows validation returned no graph".to_string();
        return Outcome::single_log(
            FlowValidation {
                valid: false,
                errors: vec![error.clone()],
                error_details: vec![FlowValidationError {
                    code: "unparseable_graph".into(),
                    message: error,
                    node_id: None,
                    field: None,
                }],
                warnings: Vec::new(),
            },
            "flow validation failed",
        );
    }
    let warnings = report.warnings;
    Outcome::single_log(
        FlowValidation {
            valid: true,
            errors: Vec::new(),
            error_details: Vec::new(),
            warnings,
        },
        "flow validated",
    )
}

/// Imports and validates native or n8n graph JSON through TinyFlows without
/// persisting it.
pub async fn flows_import(
    _ctx: &DispatchCtx,
    config: &Config,
    graph_json: Value,
    format: Option<String>,
) -> Result<Outcome<FlowImport>, String> {
    let requested = format
        .as_deref()
        .unwrap_or("auto")
        .trim()
        .to_ascii_lowercase();
    if !matches!(
        requested.as_str(),
        "auto" | "" | "native" | "tinyflows" | "n8n"
    ) {
        return Err(format!(
            "unknown import format '{requested}' (expected 'native' or 'n8n')"
        ));
    }
    let proxy = crate::modules::tinyflows::authoring_proxy(config).await?;
    let imported: ImportGraphResult = proxy
        .call(
            tinyflows_bus::methods::IMPORT_GRAPH,
            &(tinyflows_bus::ImportGraphRequest {
                graph: graph_json,
                format: Some(requested),
            },),
        )
        .await
        .map_err(|error| error.wire_message())?;
    if let Some(error) = imported.errors.first() {
        return Err(error.message.clone());
    }
    let graph: Value = imported
        .graph
        .ok_or_else(|| "TinyFlows import returned no graph".to_string())?;
    let warnings = imported.warnings;
    Ok(Outcome::single_log(
        FlowImport { graph, warnings },
        "flow imported",
    ))
}

/// OpenHuman RPC projection of the module's imported graph and warnings.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct FlowImport {
    pub graph: Value,
    pub warnings: Vec<String>,
}

/// Compatibility diagnostics for a portable Copilot graph payload.
pub(crate) async fn engine_compatibility_errors_json(
    ctx: &DispatchCtx,
    config: &Config,
    graph: Value,
) -> Result<Vec<String>, String> {
    config_aware_engine_compatibility_errors(ctx, config, graph).await
}
