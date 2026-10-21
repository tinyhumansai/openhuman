use serde_json::json;

use super::runner::doctor_report_body;

#[test]
fn invoke_outcome_is_unwrapped_to_the_snake_case_report_body() {
    let invoke_result = json!({
        "result": {
            "schema_version": 2,
            "findings": [],
            "summary": { "pass": 0, "info": 0, "warn": 0, "error": 0, "skipped": 0 },
            "ok": true
        },
        "logs": ["doctor report generated"]
    });

    let report = doctor_report_body(&invoke_result).expect("report result is present");

    assert_eq!(report["schema_version"], 2);
    assert!(report.get("result").is_none());
}

#[test]
fn missing_invoke_result_is_a_runtime_failure() {
    let invoke_result = json!({ "logs": [] });

    assert!(doctor_report_body(&invoke_result).is_err());
}
