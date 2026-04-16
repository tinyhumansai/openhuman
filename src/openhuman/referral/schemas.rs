use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferralClaimParams {
    code: String,
    #[serde(default)]
    device_fingerprint: Option<String>,
}

pub fn all_referral_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        referral_schemas("referral_get_stats"),
        referral_schemas("referral_claim"),
    ]
}

pub fn all_referral_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: referral_schemas("referral_get_stats"),
            handler: handle_referral_get_stats,
        },
        RegisteredController {
            schema: referral_schemas("referral_claim"),
            handler: handle_referral_claim,
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
        "referral_claim" => ControllerSchema {
            namespace: "referral",
            function: "claim",
            description:
                "Claim a referral link for the current user. Only users who have not yet subscribed are eligible.",
            inputs: vec![
                FieldSchema {
                    name: "code",
                    ty: TypeSchema::String,
                    comment: "Referral code to claim.",
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
                "Payload from POST /referral/claim (backend `data` field).",
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

fn handle_referral_claim(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<ReferralClaimParams>(params)?;
        let fp = payload
            .device_fingerprint
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        to_json(crate::openhuman::referral::claim_referral(&config, payload.code.trim(), fp).await?)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn all_referral_controller_schemas_advertises_stats_and_claim() {
        let names: Vec<_> = all_referral_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        assert_eq!(names, vec!["get_stats", "claim"]);
    }

    #[test]
    fn all_referral_registered_controllers_matches_schema_count() {
        assert_eq!(
            all_referral_registered_controllers().len(),
            all_referral_controller_schemas().len()
        );
    }

    #[test]
    fn get_stats_schema_has_no_inputs_and_required_output() {
        let s = referral_schemas("referral_get_stats");
        assert_eq!(s.namespace, "referral");
        assert!(s.inputs.is_empty());
        assert!(s.outputs.iter().all(|f| f.required));
    }

    #[test]
    fn claim_schema_requires_code_and_has_optional_fingerprint() {
        let s = referral_schemas("referral_claim");
        let code = s.inputs.iter().find(|f| f.name == "code").unwrap();
        assert!(code.required);
        let fp = s
            .inputs
            .iter()
            .find(|f| f.name == "deviceFingerprint")
            .unwrap();
        assert!(!fp.required);
    }

    #[test]
    fn unknown_function_returns_unknown_placeholder() {
        let s = referral_schemas("no_such");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.namespace, "referral");
    }

    #[test]
    fn claim_params_parse_camel_case_device_fingerprint() {
        let p: ReferralClaimParams = serde_json::from_value(json!({
            "code": "ABC123",
            "deviceFingerprint": "fp-xyz"
        }))
        .unwrap();
        assert_eq!(p.code, "ABC123");
        assert_eq!(p.device_fingerprint.as_deref(), Some("fp-xyz"));
    }

    #[test]
    fn claim_params_tolerate_missing_device_fingerprint() {
        let p: ReferralClaimParams = serde_json::from_value(json!({"code": "ABC"})).unwrap();
        assert!(p.device_fingerprint.is_none());
    }

    #[test]
    fn claim_params_require_code() {
        let err = serde_json::from_value::<ReferralClaimParams>(json!({})).unwrap_err();
        assert!(err.to_string().contains("code"));
    }

    #[test]
    fn deserialize_params_reports_invalid_params_prefix_on_bad_types() {
        let mut m = Map::new();
        m.insert("code".into(), json!(42));
        let err = deserialize_params::<ReferralClaimParams>(m).unwrap_err();
        assert!(err.starts_with("invalid params"));
    }

    #[test]
    fn json_output_builds_required_json_field() {
        let f = json_output("x", "c");
        assert!(f.required);
        assert!(matches!(f.ty, TypeSchema::Json));
    }

    #[test]
    fn to_json_wraps_result_and_logs() {
        let v = to_json(RpcOutcome::single_log(json!({"ok": true}), "log")).unwrap();
        assert!(v.get("result").is_some() || v.get("logs").is_some());
    }
}
