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
