use super::*;
use serde_json::{json, Value};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn report_reuses_launch_and_schema_parameter_parsing_without_global_overrides() {
    let invocation = parse(&args(&[
        "--model",
        "transient-model",
        "--provider=transient-provider",
        "doctor",
        "report",
        "--tier",
        "static",
        "--only",
        r#"["config/parse"]"#,
        "--profile",
        "selected-profile",
    ]))
    .expect("valid legacy report arguments")
    .expect("legacy report is routed through inspection");

    assert_eq!(invocation.method, "openhuman.doctor_report");
    assert_eq!(
        Value::Object(invocation.params),
        json!({
            "tier": "static",
            "only": ["config/parse"],
            "profile": "selected-profile"
        })
    );
}

#[test]
fn models_parses_legacy_bool_argument_and_keeps_raw_method() {
    let invocation = parse(&args(&[
        "-m",
        "ignored-model-override",
        "-p",
        "ignored-provider-override",
        "doctor",
        "models",
        "--use-cache",
        "false",
    ]))
    .expect("valid legacy models arguments")
    .expect("legacy models is routed through inspection");

    assert_eq!(invocation.method, "openhuman.doctor_models");
    assert_eq!(
        Value::Object(invocation.params),
        json!({"use_cache": false})
    );
}

#[test]
fn help_and_other_namespaces_remain_with_the_general_cli_dispatcher() {
    for invocation in [
        args(&["doctor", "report", "--help"]),
        args(&["doctor", "models", "help"]),
        args(&["doctor", "unknown"]),
        args(&["cron", "list"]),
        args(&["--model", "example", "cron", "list"]),
    ] {
        assert_eq!(parse(&invocation).unwrap(), None, "{invocation:?}");
    }
}

#[test]
fn legacy_doctor_parameter_errors_use_the_registered_schema() {
    let error = parse(&args(&["doctor", "report", "--unknown", "value"]))
        .expect_err("unknown controller parameters are refused");
    assert!(error.to_string().contains("unknown param 'unknown'"));
}

#[test]
fn raw_json_printer_preserves_both_outcome_and_bare_controller_shapes() {
    let report = json!({
        "result": {"schema_version": 2, "findings": []},
        "logs": ["doctor report generated"]
    });
    let models = json!({"entries": [], "summary": {"ok": 0, "skipped": 0}});

    assert_eq!(
        serde_json::from_str::<Value>(&format_json(&report).expect("format report JSON"))
            .expect("parse report JSON"),
        report
    );
    assert_eq!(
        serde_json::from_str::<Value>(&format_json(&models).expect("format models JSON"))
            .expect("parse models JSON"),
        models
    );
}
