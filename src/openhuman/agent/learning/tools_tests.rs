use super::*;
use crate::openhuman::tools::traits::ToolScope;

#[test]
fn names_and_levels() {
    assert_eq!(LearningListFacetsTool.name(), "learning_list_facets");
    assert_eq!(
        LearningListFacetsTool.permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(
        LearningUpdateFacetTool.permission_level(),
        PermissionLevel::Write
    );
    assert_eq!(
        LearningRebuildCacheTool.permission_level(),
        PermissionLevel::Execute
    );
    assert_eq!(
        LearningResetCacheTool.permission_level(),
        PermissionLevel::Dangerous
    );
    assert!(LearningEnrichProfileTool.external_effect_with_args(&serde_json::Value::Null));
    assert_eq!(LearningListFacetsTool.scope(), ToolScope::All);
}

#[test]
fn full_key_composes_class_and_suffix() {
    assert_eq!(full_key("style", "verbosity"), "style/verbosity");
}

#[tokio::test]
async fn get_facet_requires_class_and_key() {
    let err = LearningGetFacetTool
        .execute(json!({ "class": "style" }))
        .await
        .expect_err("missing key");
    assert!(err.to_string().contains("key"));
}

#[tokio::test]
async fn update_facet_requires_value() {
    let err = LearningUpdateFacetTool
        .execute(json!({ "class": "style", "key": "verbosity" }))
        .await
        .expect_err("missing value");
    assert!(err.to_string().contains("value"));
}

// ── strict class validation (#6077) ───────────────────────────────────────────
//
// `validate_class` runs before `get_cache`, so an unknown class is rejected
// without a bound memory guard — which is what lets these run as unit tests.

#[test]
fn validate_class_accepts_taxonomy_and_rejects_unknown() {
    assert!(validate_class("style").is_ok());
    assert!(validate_class("channel").is_ok());
    let err = validate_class("nonsense").expect_err("unknown class must be rejected");
    assert!(err.to_string().contains("invalid class `nonsense`"));
}

#[tokio::test]
async fn list_facets_tool_rejects_unknown_class() {
    let err = LearningListFacetsTool
        .execute(json!({ "class": "nonsense" }))
        .await
        .expect_err("unknown class must be an error, not an empty list");
    assert!(err.to_string().contains("invalid class `nonsense`"));
}

#[tokio::test]
async fn get_facet_tool_rejects_unknown_class() {
    let err = LearningGetFacetTool
        .execute(json!({ "class": "nonsense", "key": "verbosity" }))
        .await
        .expect_err("unknown class must be rejected before store access");
    assert!(err.to_string().contains("invalid class `nonsense`"));
}

#[tokio::test]
async fn forget_facet_tool_rejects_unknown_class() {
    let err = LearningForgetFacetTool
        .execute(json!({ "class": "nonsense", "key": "verbosity" }))
        .await
        .expect_err("unknown class must be rejected before store access");
    assert!(err.to_string().contains("invalid class `nonsense`"));
}
