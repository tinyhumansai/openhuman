use super::*;

#[test]
fn lookup_returns_expected_capability() {
    let capability = lookup("local_ai.download_model").expect("capability should exist");
    assert_eq!(capability.category, CapabilityCategory::LocalAI);
    assert_eq!(capability.status, CapabilityStatus::Beta);
}

#[test]
fn composio_direct_mode_capabilities_are_registered() {
    // PR #1710 PR3: ensure the direct-mode capability and the trigger-gap
    // capability are advertised in the catalog so downstream UI surfaces
    // (settings search, /about catalog dump) can find them.
    let direct = lookup("composio.direct_mode").expect("direct_mode entry exists");
    assert_eq!(direct.category, CapabilityCategory::Skills);
    // Direct mode itself is Beta (works for tool execution today).
    assert_eq!(direct.status, CapabilityStatus::Beta);

    let gap = lookup("composio.direct_mode_triggers_gap").expect("trigger-gap entry exists");
    // The trigger-webhook gap is explicitly ComingSoon to flag the
    // limitation to users browsing the capability catalog.
    assert_eq!(gap.status, CapabilityStatus::ComingSoon);
    // Both capabilities live in the same category so the settings search
    // surface groups them together consistently.
    assert_eq!(gap.category, direct.category);
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
        "local_ai.configure_provider",
        "meet.join_call",
        "meet_agent.live_loop",
        "intelligence.mcp_server",
        "intelligence.searxng_search",
        "intelligence.tool_registry",
        "intelligence.vault_markdown_writes",
        "intelligence.embedding_provider_config",
        "intelligence.embedding_provider_test",
        "conversation.subagent_mascots",
    ] {
        assert!(
            ids.contains(expected),
            "missing catalog capability `{expected}`"
        );
    }
}

/// The two embeddings entries surface a Settings-side configuration panel.
/// They share the same domain (`embeddings`) but are listed under the
/// Intelligence umbrella so they sit next to memory_tree_retrieval / mcp_server
/// in the in-app feature catalog. Pinning the relationships here defends
/// against an inadvertent recategorisation that would split them across the
/// UI's tab grouping.
#[test]
fn embedding_provider_capabilities_share_domain_and_category() {
    let config = lookup("intelligence.embedding_provider_config")
        .expect("embedding_provider_config registered");
    let test =
        lookup("intelligence.embedding_provider_test").expect("embedding_provider_test registered");

    assert_eq!(config.domain, "embeddings");
    assert_eq!(test.domain, "embeddings");
    assert_eq!(
        config.category, test.category,
        "both embedding capabilities must land in the same UI category"
    );

    // The Settings panel they describe is the same one — make sure the
    // `how_to` strings point at it, not at an out-of-date breadcrumb.
    assert!(
        config.how_to.contains("Settings") && config.how_to.contains("Embeddings"),
        "config how_to must mention Settings > … > Embeddings, got: {}",
        config.how_to
    );
    assert!(
        test.how_to.contains("Settings") && test.how_to.contains("Embeddings"),
        "test how_to must mention Settings > … > Embeddings, got: {}",
        test.how_to
    );
}

/// Privacy annotations must split cleanly: the config side touches only the
/// local keyring (LOCAL_CREDENTIALS — leaves_device=false), the test side
/// fires a probe at the configured provider (leaves_device=true). Without
/// this split, a single `None` privacy flag would force the UI to treat the
/// embeddings panel as "unknown" and the Privacy surface would under-report
/// where data goes when the test button gets clicked.
#[test]
fn embedding_provider_capabilities_split_privacy_correctly() {
    let config = lookup("intelligence.embedding_provider_config")
        .expect("embedding_provider_config registered");
    let test =
        lookup("intelligence.embedding_provider_test").expect("embedding_provider_test registered");

    let config_privacy = config
        .privacy
        .expect("config capability has privacy annotation");
    assert!(
        !config_privacy.leaves_device,
        "configuration writes only to local keyring; nothing should leave the device"
    );

    let test_privacy = test
        .privacy
        .expect("test capability has privacy annotation");
    assert!(
        test_privacy.leaves_device,
        "test fires a probe at the configured provider — must report as leaves_device"
    );
}

/// The Test Connection probe can hit any of the configured providers, not
/// just the managed cloud default. Pinning the destinations list defends
/// the Privacy surface against silently shrinking back to a single
/// destination — that's the exact under-reporting failure flagged in #2656
/// review (CodeRabbit + @graycyrus both pointed at the same line).
#[test]
fn embedding_provider_test_destinations_cover_all_providers() {
    let cap =
        lookup("intelligence.embedding_provider_test").expect("embedding_provider_test registered");
    let privacy = cap.privacy.expect("test capability has privacy annotation");

    // Joining the destinations into a single haystack so the assertions
    // tolerate cosmetic punctuation changes (parens, suffixes) but still
    // catch a destination genuinely going missing.
    let haystack = privacy.destinations.join(" | ").to_lowercase();

    for needle in ["openhuman", "openai", "cohere"] {
        assert!(
            haystack.contains(needle),
            "test probe destinations must list `{needle}` — without it the \
             Privacy surface under-reports when that provider is selected. \
             Current destinations: {:?}",
            privacy.destinations
        );
    }
    // The "custom OpenAI-compatible" path is a real provider option in
    // #2583 — listed as `custom:<url>` in `create_embedding_provider`.
    assert!(
        haystack.contains("custom") || haystack.contains("user-configured"),
        "test probe destinations must acknowledge user-configured custom \
         endpoints. Current destinations: {:?}",
        privacy.destinations
    );

    // Belt-and-braces: at least 4 distinct destinations (managed +
    // openai + cohere + custom). A drop below this means someone
    // collapsed entries.
    assert!(
        privacy.destinations.len() >= 4,
        "expected ≥4 destinations covering managed + openai + cohere + custom, \
         got {}: {:?}",
        privacy.destinations.len(),
        privacy.destinations
    );
}
