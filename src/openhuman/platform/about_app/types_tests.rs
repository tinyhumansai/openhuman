use super::*;

#[test]
fn category_serializes_expected_wire_names() {
    assert_eq!(
        serde_json::to_string(&CapabilityCategory::LocalAI).expect("serialize LocalAI"),
        "\"local_ai\""
    );
}

#[test]
fn status_serializes_expected_wire_names() {
    assert_eq!(
        serde_json::to_string(&CapabilityStatus::ComingSoon).expect("serialize ComingSoon"),
        "\"coming_soon\""
    );
}

#[test]
fn category_all_has_10_variants() {
    assert_eq!(CapabilityCategory::ALL.len(), 10);
}

#[test]
fn category_as_str_roundtrips_through_from_str() {
    for cat in CapabilityCategory::ALL {
        let s = cat.as_str();
        let parsed: CapabilityCategory = s.parse().unwrap();
        assert_eq!(parsed, cat);
    }
}

#[test]
fn category_from_str_accepts_aliases() {
    assert_eq!(
        "local-ai".parse::<CapabilityCategory>().unwrap(),
        CapabilityCategory::LocalAI
    );
    assert_eq!(
        "local ai".parse::<CapabilityCategory>().unwrap(),
        CapabilityCategory::LocalAI
    );
    assert_eq!(
        "localai".parse::<CapabilityCategory>().unwrap(),
        CapabilityCategory::LocalAI
    );
}

#[test]
fn category_from_str_is_case_insensitive() {
    assert_eq!(
        "CONVERSATION".parse::<CapabilityCategory>().unwrap(),
        CapabilityCategory::Conversation
    );
    assert_eq!(
        "  Team  ".parse::<CapabilityCategory>().unwrap(),
        CapabilityCategory::Team
    );
}

#[test]
fn category_from_str_rejects_unknown() {
    let err = "bogus".parse::<CapabilityCategory>().unwrap_err();
    assert!(err.contains("unknown capability category"));
    assert!(err.contains("bogus"));
}

#[test]
fn status_as_str_covers_all_variants() {
    assert_eq!(CapabilityStatus::Stable.as_str(), "stable");
    assert_eq!(CapabilityStatus::Beta.as_str(), "beta");
    assert_eq!(CapabilityStatus::ComingSoon.as_str(), "coming_soon");
    assert_eq!(CapabilityStatus::Deprecated.as_str(), "deprecated");
}

#[test]
fn status_serde_roundtrip() {
    for status in [
        CapabilityStatus::Stable,
        CapabilityStatus::Beta,
        CapabilityStatus::ComingSoon,
        CapabilityStatus::Deprecated,
    ] {
        let json = serde_json::to_string(&status).unwrap();
        let back: CapabilityStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, status);
    }
}

#[test]
fn category_serde_roundtrip_all() {
    for cat in CapabilityCategory::ALL {
        let json = serde_json::to_string(&cat).unwrap();
        let back: CapabilityCategory = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cat);
    }
}
