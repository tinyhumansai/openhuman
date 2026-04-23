use super::*;

#[test]
fn lookup_returns_expected_capability() {
    let capability = lookup("local_ai.download_model").expect("capability should exist");
    assert_eq!(capability.category, CapabilityCategory::LocalAI);
    assert_eq!(capability.status, CapabilityStatus::Beta);
}

#[test]
fn search_matches_keyword_across_multiple_fields() {
    let matches = search("invite");
    let ids: Vec<&str> = matches.iter().map(|capability| capability.id).collect();

    assert!(ids.contains(&"team.join_via_invite_code"));
    assert!(ids.contains(&"team.generate_invite_codes"));
    assert!(ids.contains(&"team.track_invite_usage"));
}

#[test]
fn capability_ids_are_unique() {
    let ids: BTreeSet<&str> = all_capabilities()
        .iter()
        .map(|capability| capability.id)
        .collect();
    assert_eq!(ids.len(), all_capabilities().len());
}

#[test]
fn category_filter_returns_matching_entries() {
    let capabilities = capabilities_by_category(CapabilityCategory::Automation);
    assert!(capabilities
        .iter()
        .all(|capability| { capability.category == CapabilityCategory::Automation }));
    assert!(!capabilities.is_empty());
}

#[test]
fn annotated_capability_exposes_privacy_metadata() {
    let cap = lookup("conversation.send_text").expect("capability exists");
    let privacy = cap.privacy.expect("conversation.send_text annotated");
    assert!(privacy.leaves_device);
    assert_eq!(privacy.data_kind, PrivacyDataKind::Derived);
    assert!(privacy.destinations.contains(&"OpenHuman backend"));
}

#[test]
fn local_only_capability_marks_no_destinations() {
    let cap = lookup("local_ai.embed_text").expect("capability exists");
    let privacy = cap.privacy.expect("local_ai.embed_text annotated");
    assert!(!privacy.leaves_device);
    assert_eq!(privacy.data_kind, PrivacyDataKind::Raw);
    assert!(privacy.destinations.is_empty());
}

#[test]
fn unannotated_capability_serializes_without_privacy_field() {
    let cap = lookup("conversation.create").expect("capability exists");
    assert!(cap.privacy.is_none());
    let json = serde_json::to_value(cap).expect("serialize capability");
    assert!(
        json.get("privacy").is_none(),
        "privacy field must be omitted when None: {json}"
    );
}

#[test]
fn catalog_includes_additional_user_facing_surfaces() {
    let ids: BTreeSet<&str> = all_capabilities()
        .iter()
        .map(|capability| capability.id)
        .collect();

    for expected in [
        "skills.open_connections_hub",
        "skills.connect_google",
        "auth.backup_recovery_phrase",
        "auth.configure_tool_access",
        "settings.manage_service",
        "settings.clear_app_data",
    ] {
        assert!(
            ids.contains(expected),
            "missing catalog capability `{expected}`"
        );
    }
}
