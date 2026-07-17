use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing::warn;

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
                ty: TypeSchema::String,
                required: true,
            },
            FieldSchema {
                name: "name",
                comment: "Name of the metric (e.g., 'user-feedback' or 'triage-quality').",
                ty: TypeSchema::String,
                required: true,
            },
            FieldSchema {
                name: "value",
                comment: "Numerical score value.",
                ty: TypeSchema::F64,
                required: true,
            },
            FieldSchema {
                name: "comment",
                comment: "Optional explanation of the score.",
                ty: TypeSchema::String,
                required: false,
            },
        ],
        outputs: vec![FieldSchema {
            name: "ok",
            comment: "Always true.",
            ty: TypeSchema::Bool,
            required: true,
        }],
    }]
}

pub fn all_progress_tracing_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: all_progress_tracing_controller_schemas()
            .into_iter()
            .next()
            .unwrap(),
        handler: handle_submit_score,
    }]
}

fn handle_submit_score(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = serde_json::from_value::<SubmitScoreRequest>(Value::Object(params))
            .map_err(|e| format!("Invalid SubmitScoreRequest: {}", e))?;

        let config = Config::load_or_init().await.map_err(|e| e.to_string())?;
        if let Err(e) = langfuse::push_score(
            &config,
            &req.trace_id,
            &req.name,
            req.value,
            req.comment.as_deref(),
        )
        .await
        {
            warn!(
                target: "openhuman::agent::progress_tracing",
                "[progress_tracing] push_score failed for trace {}: {}",
                req.trace_id,
                e,
            );
        }

        Ok(serde_json::to_value(SubmitScoreResponse { ok: true }).unwrap())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn observability_submit_score_schema_matches() {
        let schemas = all_progress_tracing_controller_schemas();
        assert_eq!(schemas.len(), 1);

        let s = &schemas[0];
        assert_eq!(s.namespace, "observability");
        assert_eq!(s.function, "submit_score");

        assert_eq!(s.inputs.len(), 4);
        assert_eq!(s.inputs[0].name, "trace_id");
        assert_eq!(s.inputs[1].name, "name");
        assert_eq!(s.inputs[2].name, "value");
        assert_eq!(s.inputs[3].name, "comment");

        assert_eq!(s.inputs[0].ty, TypeSchema::String);
        assert_eq!(s.inputs[1].ty, TypeSchema::String);
        assert_eq!(s.inputs[2].ty, TypeSchema::F64);
        assert_eq!(s.inputs[3].ty, TypeSchema::String);

        assert!(s.inputs[0].required);
        assert!(s.inputs[1].required);
        assert!(s.inputs[2].required);
        assert!(!s.inputs[3].required);

        assert_eq!(s.outputs.len(), 1);
        assert_eq!(s.outputs[0].name, "ok");
        assert_eq!(s.outputs[0].ty, TypeSchema::Bool);
    }

    #[test]
    fn observability_submit_score_handler_is_registered() {
        let controllers = all_progress_tracing_registered_controllers();
        assert_eq!(controllers.len(), 1);
    }

    #[test]
    fn submit_score_request_deserializes_from_json() {
        let req: SubmitScoreRequest = serde_json::from_value(json!({
            "trace_id": "trace-1",
            "name": "user-feedback",
            "value": 0.75,
        }))
        .unwrap();
        assert_eq!(req.trace_id, "trace-1");
        assert_eq!(req.name, "user-feedback");
        assert_eq!(req.value, 0.75);
        assert_eq!(req.comment, None);
    }

    #[test]
    fn submit_score_request_deserializes_with_optional_comment() {
        let req: SubmitScoreRequest = serde_json::from_value(json!({
            "trace_id": "trace-1",
            "name": "user-feedback",
            "value": 1.0,
            "comment": "Great response",
        }))
        .unwrap();
        assert_eq!(req.comment, Some("Great response".to_string()));
    }

    #[test]
    fn submit_score_response_serializes_ok_true() {
        let resp = SubmitScoreResponse { ok: true };
        let v = serde_json::to_value(resp).unwrap();
        assert_eq!(v, json!({ "ok": true }));
    }

    #[tokio::test]
    async fn test_handle_submit_score_success() {
        std::env::set_var("OPENHUMAN_SHARE_USAGE_DATA", "false"); // Disable real network

        let params = json!({
            "trace_id": "trace-test",
            "name": "user-feedback",
            "value": 1.0,
        })
        .as_object()
        .unwrap()
        .clone();

        let result = handle_submit_score(params).await.unwrap();
        assert_eq!(result, json!({ "ok": true }));
    }

    #[tokio::test]
    async fn test_handle_submit_score_invalid_params() {
        let params = json!({
            "trace_id": "trace-test",
        })
        .as_object()
        .unwrap()
        .clone();

        let err = handle_submit_score(params).await.unwrap_err();
        assert!(err.to_string().contains("Invalid SubmitScoreRequest"));
    }
}
