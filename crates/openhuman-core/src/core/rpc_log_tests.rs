use super::*;

#[test]
fn structured_redactor_output_is_parsed_back_as_json() {
    let params = serde_json::json!({
        "provider": { "api_key": "secret-value", "model": "small" },
        "items": [{ "authorization": "Bearer secret-value" }]
    });

    let output = redact_params_with(&params, |encoded| {
        let mut value: Value = serde_json::from_str(encoded).unwrap();
        value["provider"]["api_key"] = Value::String("[REDACTED]".into());
        value["items"][0]["authorization"] = Value::String("[REDACTED]".into());
        serde_json::to_string(&value).unwrap()
    });

    assert_eq!(output["provider"]["api_key"], "[REDACTED]");
    assert_eq!(output["provider"]["model"], "small");
    assert_eq!(output["items"][0]["authorization"], "[REDACTED]");
}

#[test]
fn unavailable_or_malformed_structured_redaction_suppresses_params() {
    let params = serde_json::json!({ "token": "must-not-leak" });

    let unavailable = redact_params_with(&params, |_| {
        crate::core::log_redaction::SUPPRESSED.to_owned()
    });
    let malformed = redact_params_with(&params, |_| "not-json".to_owned());

    assert_eq!(
        unavailable,
        Value::String(crate::core::log_redaction::SUPPRESSED.into())
    );
    assert_eq!(
        malformed,
        Value::String(crate::core::log_redaction::SUPPRESSED.into())
    );
}

#[cfg(not(feature = "security-module"))]
#[test]
fn disabled_security_module_suppresses_rpc_params() {
    let params = serde_json::json!({ "token": "must-not-leak" });
    assert_eq!(
        redact_params_for_log(&params),
        Value::String(crate::core::log_redaction::SUPPRESSED.into())
    );
}
