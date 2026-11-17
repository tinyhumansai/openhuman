use super::*;

#[tokio::test]
async fn readonly_tier_refusal_keeps_one_capability_envelope_before_curation() {
    use crate::security::AutonomyLevel;

    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = dir.path().join("config.toml");
    config.workspace_dir = dir.path().join("workspace");
    config.action_dir = dir.path().join("action");
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    std::fs::create_dir_all(&config.action_dir).unwrap();
    config.autonomy.enabled = true;
    config.autonomy.level = AutonomyLevel::ReadOnly;
    config.save().await.unwrap();

    let dispatch =
        crate::core::runtime::DispatchCtx::new(crate::core::runtime::CoreContext::for_test(
            crate::core::runtime::DomainSet::full(),
            Some(config.workspace_dir.clone()),
        ));
    let security =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    let approval = ApprovalTurnContext::default();
    let call = ToolCallCtx {
        dispatch: &dispatch,
        config: &config,
        security: &security,
        approval: &approval,
    };
    let error = ComposioToolBackend
        .invoke(&call, "UNRECOGNIZED_ACTION", Value::Null, None)
        .await
        .unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("ReadOnly"), "{rendered}");
    assert!(
        rendered.contains("Network-class actions are blocked"),
        "{rendered}"
    );
    assert!(
        !rendered.contains("tool not permitted"),
        "tier gate must precede curation"
    );
    assert_eq!(
        rendered.matches("capability error:").count(),
        1,
        "{rendered}"
    );
}

#[test]
fn capability_payload_and_audit_display_match_the_original_error_envelope() {
    let payload = capability_payload(EngineError::Capability("blocked by autonomy tier".into()));
    assert_eq!(payload, "blocked by autonomy tier");
    assert_eq!(
        capability_error_display(&payload),
        EngineError::Capability("blocked by autonomy tier".into()).to_string()
    );
}
