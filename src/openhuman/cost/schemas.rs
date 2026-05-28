//! Controller schemas and JSON-RPC dispatchers for the cost dashboard.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::rpc as cost_rpc;

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DailyHistoryParams {
    #[serde(default)]
    days: Option<u32>,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schema_for("cost_get_dashboard"),
        schema_for("cost_get_daily_history"),
        schema_for("cost_get_summary"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema_for("cost_get_dashboard"),
            handler: handle_cost_get_dashboard,
        },
        RegisteredController {
            schema: schema_for("cost_get_daily_history"),
            handler: handle_cost_get_daily_history,
        },
        RegisteredController {
            schema: schema_for("cost_get_summary"),
            handler: handle_cost_get_summary,
        },
    ]
}

fn schema_for(function: &str) -> ControllerSchema {
    match function {
        "cost_get_dashboard" => ControllerSchema {
            namespace: "cost",
            function: "get_dashboard",
            description:
                "Fetch the 7-day cost & token dashboard payload: per-day buckets, summary \
                 metrics, budget utilisation, and per-model breakdown.",
            inputs: vec![],
            outputs: vec![json_output(
                "dashboard",
                "Dashboard payload with `days`, `byModel`, summary fields and budget status.",
            )],
        },
        "cost_get_daily_history" => ControllerSchema {
            namespace: "cost",
            function: "get_daily_history",
            description: "Fetch a per-day cost/token history for the requested span (default 7 \
                          days, clamped to [1, 366]).",
            inputs: vec![FieldSchema {
                name: "days",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Number of trailing days to include (default 7).",
                required: false,
            }],
            outputs: vec![json_output(
                "entries",
                "Ordered list of daily entries, oldest first; gaps zero-filled.",
            )],
        },
        "cost_get_summary" => ControllerSchema {
            namespace: "cost",
            function: "get_summary",
            description: "Fetch the live session / daily / monthly cost summary.",
            inputs: vec![],
            outputs: vec![json_output(
                "summary",
                "Aggregated cost & token usage for the current session and active period.",
            )],
        },
        _ => ControllerSchema {
            namespace: "cost",
            function: "unknown",
            description: "Unknown cost controller.",
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

/// Short opaque correlation id for log threading across an async handler
/// invocation. Eight hex chars are enough to disambiguate concurrent
/// dashboard polls without bloating log lines, and the value is local
/// so it does not leak across processes.
fn new_correlation_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn handle_cost_get_dashboard(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_dashboard.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_dashboard.config_load_failed err={err}");
        })?;
        let outcome = cost_rpc::dashboard(&config).map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_dashboard.error err={s}");
            s
        })?;
        let json = to_json(outcome);
        log::debug!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_dashboard.exit ok={}", json.is_ok());
        json
    })
}

fn handle_cost_get_daily_history(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_daily_history.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_daily_history.config_load_failed err={err}");
        })?;
        let payload = if params.is_empty() {
            DailyHistoryParams::default()
        } else {
            serde_json::from_value::<DailyHistoryParams>(Value::Object(params)).map_err(|e| {
                let s = format!("invalid params: {e}");
                log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_daily_history.bad_params err={s}");
                s
            })?
        };
        let days = payload.days.unwrap_or(7);
        let outcome = cost_rpc::daily_history(&config, days).map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_daily_history.error err={s}");
            s
        })?;
        let json = to_json(outcome);
        log::debug!(
            target: "cost_rpc",
            "[cost_rpc][{cid}] cost_get_daily_history.exit days={days} ok={}",
            json.is_ok()
        );
        json
    })
}

fn handle_cost_get_summary(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_summary.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_summary.config_load_failed err={err}");
        })?;
        let outcome = cost_rpc::summary(&config).map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_summary.error err={s}");
            s
        })?;
        let json = to_json(outcome);
        log::debug!(target: "cost_rpc", "[cost_rpc][{cid}] cost_get_summary.exit ok={}", json.is_ok());
        json
    })
}

fn to_json(outcome: RpcOutcome<Value>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_controller_schemas_lists_three_functions() {
        let schemas = all_controller_schemas();
        let names: Vec<&'static str> = schemas.iter().map(|s| s.function).collect();
        assert_eq!(schemas.len(), 3);
        assert!(names.contains(&"get_dashboard"));
        assert!(names.contains(&"get_daily_history"));
        assert!(names.contains(&"get_summary"));
        for schema in &schemas {
            assert_eq!(schema.namespace, "cost");
        }
    }

    #[test]
    fn all_registered_controllers_has_three_handlers_matching_schemas() {
        let registered = all_registered_controllers();
        assert_eq!(registered.len(), 3);
        let schema_fns: Vec<&'static str> = registered.iter().map(|r| r.schema.function).collect();
        assert!(schema_fns.contains(&"get_dashboard"));
        assert!(schema_fns.contains(&"get_daily_history"));
        assert!(schema_fns.contains(&"get_summary"));
    }

    #[test]
    fn schema_for_dashboard_has_no_inputs_and_one_output() {
        let s = schema_for("cost_get_dashboard");
        assert_eq!(s.function, "get_dashboard");
        assert!(s.inputs.is_empty());
        assert_eq!(s.outputs.len(), 1);
        assert_eq!(s.outputs[0].name, "dashboard");
    }

    #[test]
    fn schema_for_daily_history_has_optional_days_input() {
        let s = schema_for("cost_get_daily_history");
        assert_eq!(s.function, "get_daily_history");
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].name, "days");
        assert!(!s.inputs[0].required);
    }

    #[test]
    fn schema_for_summary_returns_summary_output() {
        let s = schema_for("cost_get_summary");
        assert_eq!(s.function, "get_summary");
        assert_eq!(s.outputs[0].name, "summary");
    }

    #[test]
    fn schema_for_unknown_returns_error_shape() {
        let s = schema_for("cost_get_nonexistent");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.outputs[0].name, "error");
    }

    #[test]
    fn new_correlation_id_returns_eight_hex_chars() {
        let cid = new_correlation_id();
        assert_eq!(cid.len(), 8);
        assert!(cid.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn new_correlation_id_is_unique_across_calls() {
        let a = new_correlation_id();
        let b = new_correlation_id();
        // Collision probability for 8 hex chars (32 bits) per call is
        // ~1/4B — virtually zero for a unit test.
        assert_ne!(a, b);
    }
}
