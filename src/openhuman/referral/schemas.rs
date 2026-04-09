use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferralApplyParams {
    code: String,
    #[serde(default)]
    device_fingerprint: Option<String>,
}

pub fn all_referral_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        referral_schemas("referral_get_stats"),
        referral_schemas("referral_apply"),
    ]
}

pub fn all_referral_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: referral_schemas("referral_get_stats"),
            handler: handle_referral_get_stats,
        },
        RegisteredController {
            schema: referral_schemas("referral_apply"),
            handler: handle_referral_apply,
        },
    ]
}

pub fn referral_schemas(function: &str) -> ControllerSchema {
    match function {
        "referral_get_stats" => ControllerSchema {
            namespace: "referral",
            function: "get_stats",
            description:
                "Fetch referral code, link, totals, and referred-user rows from the backend.",
            inputs: vec![],
            outputs: vec![json_output(
                "stats",
                "Payload from GET /referral/stats (backend `data` field).",
            )],
        },
        "referral_apply" => ControllerSchema {
            namespace: "referral",
            function: "apply",
            description:
                "Apply a friend's referral code for the current user (backend eligibility rules).",
            inputs: vec![
                FieldSchema {
                    name: "code",
                    ty: TypeSchema::String,
                    comment: "Referral code to apply.",
                    required: true,
                },
                FieldSchema {
                    name: "deviceFingerprint",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional client fingerprint for abuse signals.",
                    required: false,
                },
            ],
            outputs: vec![json_output(
                "result",
                "Payload from POST /referral/apply (backend `data` field).",
            )],
        },
        _ => ControllerSchema {
            namespace: "referral",
            function: "unknown",
            description: "Unknown referral controller.",
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

fn handle_referral_get_stats(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::referral::get_stats(&config).await?)
    })
}

fn handle_referral_apply(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<ReferralApplyParams>(params)?;
        let fp = payload
            .device_fingerprint
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        to_json(crate::openhuman::referral::apply_code(&config, payload.code.trim(), fp).await?)
    })
}

fn to_json(outcome: RpcOutcome<Value>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}
