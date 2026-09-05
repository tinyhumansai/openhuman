use super::*;

#[test]
fn default_points_at_local_core() {
    let cfg = YouPetConfig::default();
    assert_eq!(cfg.core_api_url, DEFAULT_YOUPET_CORE_API_URL);
    assert_eq!(cfg.workbench_actor_id, DEFAULT_YOUPET_WORKBENCH_ACTOR_ID);
    assert!(cfg.service_token.is_none());
    assert!(cfg.operator_user_id.is_none());
}

#[test]
fn debug_redacts_service_token() {
    let cfg = YouPetConfig {
        service_token: Some("secret-token".into()),
        ..Default::default()
    };
    let rendered = format!("{cfg:?}");
    assert!(!rendered.contains("secret-token"));
    assert!(rendered.contains("<redacted>"));
}

#[test]
fn helpers_trim_and_default_blank_fields() {
    let cfg = YouPetConfig {
        core_api_url: "  https://core.example.test///  ".into(),
        service_token: Some("  tok  ".into()),
        workbench_actor_id: "   ".into(),
        operator_user_id: Some("  operator-1  ".into()),
        tenant_id: Some("  20000000-0000-0000-0000-000000000001  ".into()),
    };
    assert_eq!(cfg.normalized_core_api_url(), "https://core.example.test");
    assert_eq!(cfg.service_token(), Some("tok"));
    assert_eq!(cfg.workbench_actor_id(), DEFAULT_YOUPET_WORKBENCH_ACTOR_ID);
    assert_eq!(cfg.operator_user_id(), Some("operator-1"));
    assert_eq!(
        cfg.tenant_id(),
        Some("20000000-0000-0000-0000-000000000001")
    );

    let blank_operator = YouPetConfig {
        operator_user_id: Some("   ".into()),
        ..Default::default()
    };
    assert_eq!(blank_operator.operator_user_id(), None);
}
