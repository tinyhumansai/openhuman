//! Controller schemas + handlers for the dashboard domain.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

use super::ops;
use super::types::ModelHealthResponse;

pub fn all_dashboard_controller_schemas() -> Vec<ControllerSchema> {
    vec![dashboard_schemas("dashboard_model_health")]
}

pub fn all_dashboard_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: dashboard_schemas("dashboard_model_health"),
        handler: handle_dashboard_model_health,
    }]
}

pub fn dashboard_schemas(function: &str) -> ControllerSchema {
    match function {
        "dashboard_model_health" => ControllerSchema {
            namespace: "dashboard",
            function: "model_health",
            description: "Per-model health comparison rows for the desktop dashboard panel \
                 — joins local model_registry with dashboard.model_health thresholds. \
                 Telemetry-driven metric fields are placeholders until a local \
                 telemetry sink is wired in.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "models",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Object {
                        fields: model_health_entry_fields(),
                    })),
                    comment: "Per-model rows: id, provider, cost_per_1m_output, vision, \
                              quality_score, hallucination_rate, agents_using, tasks_evaluated.",
                    required: true,
                },
                FieldSchema {
                    name: "config",
                    ty: TypeSchema::Object {
                        fields: model_health_config_fields(),
                    },
                    comment: "Threshold view: hallucination_threshold, min_tasks_for_rating, \
                              evaluation_window_tasks.",
                    required: true,
                },
            ],
        },
        _ => ControllerSchema {
            namespace: "dashboard",
            function: "unknown",
            description: "Unknown dashboard controller.",
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

fn model_health_entry_fields() -> Vec<FieldSchema> {
    vec![
        FieldSchema {
            name: "id",
            ty: TypeSchema::String,
            comment: "Model identifier as configured in `model_registry`.",
            required: true,
        },
        FieldSchema {
            name: "provider",
            ty: TypeSchema::String,
            comment: "Provider label, e.g. SiliconFlow, OpenRouter.",
            required: true,
        },
        FieldSchema {
            name: "cost_per_1m_output",
            ty: TypeSchema::F64,
            comment: "USD cost per 1M output tokens from local config.",
            required: true,
        },
        FieldSchema {
            name: "vision",
            ty: TypeSchema::Bool,
            comment: "True when the model accepts image inputs.",
            required: true,
        },
        FieldSchema {
            name: "quality_score",
            ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
            comment: "Per-model quality score (placeholder — null until telemetry lands).",
            required: false,
        },
        FieldSchema {
            name: "hallucination_rate",
            ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
            comment: "Observed hallucination rate (placeholder — null until telemetry lands).",
            required: false,
        },
        FieldSchema {
            name: "agents_using",
            ty: TypeSchema::U64,
            comment: "Number of agents bound to this model (placeholder — 0 until wired).",
            required: true,
        },
        FieldSchema {
            name: "tasks_evaluated",
            ty: TypeSchema::U64,
            comment: "Tasks evaluated against this model (placeholder — 0 until wired).",
            required: true,
        },
    ]
}

fn model_health_config_fields() -> Vec<FieldSchema> {
    vec![
        FieldSchema {
            name: "hallucination_threshold",
            ty: TypeSchema::F64,
            comment: "Rate above which a model is flagged `replace`.",
            required: true,
        },
        FieldSchema {
            name: "min_tasks_for_rating",
            ty: TypeSchema::U64,
            comment: "Minimum tasks evaluated before quality/hallucination are considered.",
            required: true,
        },
        FieldSchema {
            name: "evaluation_window_tasks",
            ty: TypeSchema::U64,
            comment: "Sliding window size used when computing telemetry metrics.",
            required: true,
        },
    ]
}

fn handle_dashboard_model_health(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[dashboard] model_health request received");
        let cfg = crate::openhuman::config::rpc::load_config_with_timeout()
            .await
            .map_err(|err| {
                log::warn!("[dashboard] model_health failed to load config: {err}");
                format!("config unavailable: {err}")
            })?;
        let outcome: RpcOutcome<ModelHealthResponse> = ops::model_health(&cfg)?;
        outcome.into_cli_compatible_json()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_namespace_and_function_are_stable() {
        let s = dashboard_schemas("dashboard_model_health");
        assert_eq!(s.namespace, "dashboard");
        assert_eq!(s.function, "model_health");
    }

    #[test]
    fn controller_lists_match_lengths() {
        assert_eq!(
            all_dashboard_controller_schemas().len(),
            all_dashboard_registered_controllers().len()
        );
    }
}
