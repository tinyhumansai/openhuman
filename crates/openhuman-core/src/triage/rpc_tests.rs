use super::*;
use serde_json::json;

#[test]
fn triage_evaluate_schema_exposes_expected_inputs() {
    let triage = triage_evaluate_schema();
    assert_eq!(triage.namespace, "agent");
    assert_eq!(triage.function, "triage_evaluate");
    assert_eq!(triage.inputs.len(), 7);
    assert!(triage
        .inputs
        .iter()
        .any(|input| input.name == "payload" && input.required));
    assert!(triage
        .inputs
        .iter()
        .any(|input| input.name == "dry_run" && !input.required));
}

#[tokio::test]
async fn triage_handler_rejects_unknown_source() {
    let err = handle_triage_evaluate(
        DispatchCtx::process_default(),
        Map::from_iter([
            ("source".into(), Value::String("__unknown_source__".into())),
            ("display_label".into(), Value::String("lbl".into())),
            ("payload".into(), json!({})),
        ]),
    )
    .await
    .expect_err("unsupported source should fail before runtime dispatch");
    assert!(err.contains("unsupported trigger source"));
}
