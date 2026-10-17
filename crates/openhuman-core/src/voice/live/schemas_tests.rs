use super::*;
use crate::core::runtime::DispatchCtx;

#[test]
fn registers_four_live_controllers_in_the_voice_namespace() {
    let controllers = live_registered_controllers();
    let functions: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
    assert_eq!(
        functions,
        vec![
            "live_providers",
            "live_settings_get",
            "live_settings_set",
            "live_test_provider"
        ]
    );
    assert!(controllers.iter().all(|c| c.schema.namespace == "voice"));
    assert_eq!(live_controller_schemas().len(), 4);
    assert_eq!(live_schemas("missing").function, "unknown");
    let test = live_schemas("voice_live_test_provider");
    assert!(test
        .inputs
        .iter()
        .any(|f| f.name == "provider" && f.required));
}

#[tokio::test]
async fn handlers_reject_bad_params() {
    let bad = |v: serde_json::Value| v.as_object().unwrap().clone();
    assert!(
        handle_live_test_provider(DispatchCtx::process_default(), bad(serde_json::json!({})))
            .await
            .is_err()
    );
    assert!(handle_live_settings_set(
        DispatchCtx::process_default(),
        bad(serde_json::json!({"gemini": 5}))
    )
    .await
    .is_err());
}
