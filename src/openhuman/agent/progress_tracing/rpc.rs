use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::future::Future;
use std::pin::Pin;

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::Config;

use super::langfuse;

#[derive(Debug, Deserialize)]
pub struct SubmitScoreRequest {
    pub trace_id: String,
    pub name: String,
    pub value: f64,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SubmitScoreResponse {
    pub ok: bool,
}

pub fn all_progress_tracing_controller_schemas() -> Vec<ControllerSchema> {
    vec![ControllerSchema {
        namespace: "observability",
        function: "submit_score",
        description: "Submit a quality/latency score to Langfuse attached to a trace.",
        inputs: vec![
            FieldSchema {
                name: "trace_id",
                comment: "Trace ID to attach the score to.",
                type_schema: TypeSchema::String,
                required: true,
            },
            FieldSchema {
                name: "name",
                comment: "Name of the metric (e.g., 'user-feedback' or 'triage-quality').",
                type_schema: TypeSchema::String,
                required: true,
            },
            FieldSchema {
                name: "value",
                comment: "Numerical score value.",
                type_schema: TypeSchema::Float,
                required: true,
            },
            FieldSchema {
                name: "comment",
                comment: "Optional explanation of the score.",
                type_schema: TypeSchema::String,
                required: false,
            },
        ],
        outputs: vec![FieldSchema {
            name: "ok",
            comment: "Always true.",
            type_schema: TypeSchema::Boolean,
            required: true,
        }],
    }]
}

pub fn all_progress_tracing_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: all_progress_tracing_controller_schemas().into_iter().next().unwrap(),
        handler: handle_submit_score,
    }]
}

fn handle_submit_score(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = serde_json::from_value::<SubmitScoreRequest>(Value::Object(params))
            .map_err(|e| format!("Invalid SubmitScoreRequest: {}", e))?;
        
        let config = Config::load_or_init().await.map_err(|e| e.to_string())?;
        let _ = langfuse::push_score(
            &config,
            &req.trace_id,
            &req.name,
            req.value,
            req.comment.as_deref(),
        )
        .await;

        Ok(serde_json::to_value(SubmitScoreResponse { ok: true }).unwrap())
    })
}
