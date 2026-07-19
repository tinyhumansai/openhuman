//! Unit tests for the [`EgressDescriptor`](super::EgressDescriptor) contract
//! (privacy epic S2, #4436).

use super::*;

#[test]
fn inference_external_shape() {
    let d = EgressDescriptor::inference("openai", "gpt-4o", true);
    assert_eq!(d.provider_slug, "openai");
    assert_eq!(d.service, "gpt-4o");
    assert!(d.is_external);
    assert_eq!(d.reason, EgressReason::Inference);
    assert_eq!(d.data_kinds, vec![DataKind::Prompt]);
    // S5 risk fields default-empty until the detector lands.
    assert_eq!(d.risk_level, IdentificationRisk::Unknown);
    assert!(d.risk_categories.is_empty());
}

#[test]
fn inference_local_is_not_external() {
    let d = EgressDescriptor::inference("ollama", "llama3", false);
    assert!(!d.is_external);
    assert_eq!(d.reason, EgressReason::Inference);
}

#[test]
fn composio_shape_is_external_toolcall() {
    let d = EgressDescriptor::composio("GMAIL_SEND_EMAIL");
    assert_eq!(d.provider_slug, "composio");
    assert_eq!(d.service, "GMAIL_SEND_EMAIL");
    assert!(d.is_external);
    assert_eq!(d.reason, EgressReason::ToolCall);
    assert_eq!(d.data_kinds, vec![DataKind::ToolArguments]);
}

#[test]
fn integration_shape_targets_backend() {
    let d = EgressDescriptor::integration("/agent-integrations/composio/tools");
    assert_eq!(d.provider_slug, "openhuman_backend");
    assert_eq!(d.service, "/agent-integrations/composio/tools");
    assert!(d.is_external);
    assert_eq!(d.reason, EgressReason::Integration);
    assert_eq!(d.data_kinds, vec![DataKind::Metadata]);
}

#[test]
fn embedding_shape() {
    let d = EgressDescriptor::embedding("cloud", "embedding-v1");
    assert_eq!(d.provider_slug, "cloud");
    assert_eq!(d.service, "embedding-v1");
    assert!(d.is_external);
    assert_eq!(d.reason, EgressReason::Embedding);
    assert_eq!(d.data_kinds, vec![DataKind::EmbeddingInput]);
}

#[test]
fn network_fetch_shape() {
    let d = EgressDescriptor::network_fetch("example.com");
    assert_eq!(d.provider_slug, "network");
    assert_eq!(d.service, "example.com");
    assert!(d.is_external);
    assert_eq!(d.reason, EgressReason::NetworkFetch);
    assert_eq!(d.data_kinds, vec![DataKind::Url]);
}

#[test]
fn with_data_kind_appends_without_duplicates() {
    let d = EgressDescriptor::network_fetch("example.com")
        .with_data_kind(DataKind::ToolArguments)
        .with_data_kind(DataKind::ToolArguments); // second is a no-op
    assert_eq!(d.data_kinds, vec![DataKind::Url, DataKind::ToolArguments]);
}

#[test]
fn with_risk_populates_s5_fields() {
    let d = EgressDescriptor::inference("openai", "gpt-4o", true).with_risk(
        IdentificationRisk::High,
        vec!["email".to_string(), "phone".to_string()],
    );
    assert_eq!(d.risk_level, IdentificationRisk::High);
    assert_eq!(d.risk_categories, vec!["email", "phone"]);
}

#[test]
fn identification_risk_defaults_unknown() {
    assert_eq!(IdentificationRisk::default(), IdentificationRisk::Unknown);
}

#[test]
fn serde_round_trip_preserves_all_fields() {
    let d = EgressDescriptor::composio("GMAIL_SEND_EMAIL")
        .with_data_kind(DataKind::FileContent)
        .with_risk(IdentificationRisk::Medium, vec!["name".to_string()]);
    let json = serde_json::to_value(&d).expect("serialize");
    // Enum values serialize as snake_case for the frontend contract.
    assert_eq!(json["reason"], "tool_call");
    assert_eq!(json["risk_level"], "medium");
    assert_eq!(json["is_external"], true);
    let back: EgressDescriptor = serde_json::from_value(json).expect("deserialize");
    assert_eq!(back, d);
}

#[test]
fn deserialize_tolerates_missing_s5_risk_fields() {
    // An S2-era producer that omits the risk fields must still deserialize
    // (forward/backward compatibility with the S5 detector).
    let json = serde_json::json!({
        "provider_slug": "openai",
        "service": "gpt-4o",
        "is_external": true,
        "reason": "inference",
        "data_kinds": ["prompt"],
    });
    let d: EgressDescriptor = serde_json::from_value(json).expect("deserialize without risk");
    assert_eq!(d.risk_level, IdentificationRisk::Unknown);
    assert!(d.risk_categories.is_empty());
}
