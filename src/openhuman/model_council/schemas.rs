//! JSON-RPC controller surface for the Model Council.
//!
//! Exposes a single method, `openhuman.model_council_run`, which takes a
//! question + a list of member model ids + a chair model id, runs the council
//! (see [`crate::openhuman::model_council::council`]), and returns the
//! aggregated result synchronously in the JSON-RPC response.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
struct ModelCouncilParams {
    question: String,
    member_models: Vec<String>,
    chair_model: String,
    temperature: Option<f64>,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("run")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("run"),
        handler: handle_run,
    }]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "run" => ControllerSchema {
            namespace: "model_council",
            function: "run",
            description: "Run a question through several member models concurrently, then \
                          synthesize their answers with a chair model. Returns each member's \
                          answer (or error) plus the chair's synthesis.",
            inputs: vec![
                required_string("question", "The question to put to the council."),
                required_string_array(
                    "member_models",
                    "Member model ids to consult (deduplicated; max 5).",
                ),
                required_string(
                    "chair_model",
                    "Model id that synthesizes the member answers.",
                ),
                optional_f64(
                    "temperature",
                    "Optional sampling temperature for all calls.",
                ),
            ],
            outputs: vec![json_output(
                "result",
                "Council result: per-member answers + chair synthesis.",
            )],
        },
        _ => ControllerSchema {
            namespace: "model_council",
            function: "unknown",
            description: "Unknown model_council controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Error message.",
                required: true,
            }],
        },
    }
}

fn handle_run(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[model-council] handle_run: received RPC request");
        let p = deserialize_params::<ModelCouncilParams>(params)?;
        // Log a sanitized summary only — never the full question text.
        log::debug!(
            "[model-council] handle_run: question_len={}, members={}, chair={}",
            p.question.len(),
            p.member_models.len(),
            p.chair_model
        );
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::model_council::council::run_council(
                &config,
                &p.question,
                &p.member_models,
                &p.chair_model,
                p.temperature,
            )
            .await
            .map_err(|e| {
                log::debug!("[model-council] handle_run: run_council failed: {e}");
                e
            })?,
        )
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn required_string_array(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Array(Box::new(TypeSchema::String)),
        comment,
        required: true,
    }
}

fn optional_f64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_schema_inventory_is_stable() {
        let schemas = all_controller_schemas();
        let functions: Vec<_> = schemas.iter().map(|schema| schema.function).collect();
        assert_eq!(functions, vec!["run"]);
        assert_eq!(schemas.len(), all_registered_controllers().len());
    }

    #[test]
    fn run_schema_exposes_expected_inputs_and_method_name() {
        let run = schemas("run");
        assert_eq!(run.namespace, "model_council");
        assert_eq!(run.function, "run");
        assert_eq!(
            crate::core::all::rpc_method_name(&run),
            "openhuman.model_council_run"
        );
        assert_eq!(run.inputs.len(), 4);
        assert!(run
            .inputs
            .iter()
            .any(|input| input.name == "question" && input.required));
        let members = run
            .inputs
            .iter()
            .find(|input| input.name == "member_models")
            .expect("member_models input present");
        assert!(matches!(members.ty, TypeSchema::Array(_)));
        assert!(members.required);
        let temperature = run
            .inputs
            .iter()
            .find(|input| input.name == "temperature")
            .expect("temperature input present");
        assert!(!temperature.required);
        assert!(matches!(temperature.ty, TypeSchema::Option(_)));
    }

    #[test]
    fn unknown_function_falls_back_to_error_output() {
        let unknown = schemas("nope");
        assert_eq!(unknown.function, "unknown");
        assert_eq!(unknown.outputs[0].name, "error");
    }

    #[test]
    fn deserialize_params_parses_a_well_formed_payload() {
        let params = Map::from_iter([
            ("question".to_string(), Value::from("What is 2+2?")),
            (
                "member_models".to_string(),
                Value::from(vec![Value::from("gpt"), Value::from("claude")]),
            ),
            ("chair_model".to_string(), Value::from("chair")),
            ("temperature".to_string(), Value::from(0.5)),
        ]);
        let parsed = deserialize_params::<ModelCouncilParams>(params).unwrap();
        assert_eq!(parsed.question, "What is 2+2?");
        assert_eq!(parsed.member_models, vec!["gpt", "claude"]);
        assert_eq!(parsed.chair_model, "chair");
        assert_eq!(parsed.temperature, Some(0.5));
    }

    #[test]
    fn deserialize_params_allows_omitted_temperature() {
        let params = Map::from_iter([
            ("question".to_string(), Value::from("q")),
            (
                "member_models".to_string(),
                Value::from(vec![Value::from("a")]),
            ),
            ("chair_model".to_string(), Value::from("chair")),
        ]);
        let parsed = deserialize_params::<ModelCouncilParams>(params).unwrap();
        assert_eq!(parsed.temperature, None);
    }

    #[test]
    fn deserialize_params_rejects_missing_required_field() {
        let params = Map::from_iter([("question".to_string(), Value::from("q"))]);
        assert!(deserialize_params::<ModelCouncilParams>(params).is_err());
    }
}
