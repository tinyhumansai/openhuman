use super::*;

#[test]
fn lookup_returns_expected_capability() {
    let capability = lookup("local_ai.configure_provider").expect("capability should exist");
    assert_eq!(capability.category, CapabilityCategory::LocalAI);
    assert_eq!(capability.status, CapabilityStatus::Beta);
}

/// PR #3090: the global push-to-talk feature is user-facing and must be
/// discoverable in the capability catalog so the in-app /about surface and
/// settings search can describe it. Pins the id, category, and the rough
/// shape of the how_to / description so a future rewrite can't silently
/// drop the entry or split it from the Conversation umbrella where the
/// related voice capabilities live.
#[test]
fn capability_list_includes_voice_ptt() {
    let caps = all_capabilities();
    assert!(
        caps.iter().any(|c| c.id == "voice.ptt"),
        "voice.ptt capability must be registered"
    );

    let ptt = lookup("voice.ptt").expect("voice.ptt should be registered");
    assert_eq!(ptt.category, CapabilityCategory::Conversation);
    assert_eq!(ptt.domain, "voice");
    assert!(
        ptt.how_to.contains("Push-to-Talk") || ptt.how_to.contains("push-to-talk"),
        "how_to must mention Push-to-Talk, got: {}",
        ptt.how_to
    );
    assert!(
        ptt.description.to_lowercase().contains("hold")
            && ptt.description.to_lowercase().contains("hotkey"),
        "description must describe the hold-to-talk hotkey behaviour, got: {}",
        ptt.description
    );
}

#[test]
fn composio_direct_mode_capabilities_are_registered() {
    // PR #1710 PR3: ensure the direct-mode capability and the trigger-gap
    // capability are advertised in the catalog so downstream UI surfaces
    // (settings search, /about catalog dump) can find them.
    let direct = lookup("composio.direct_mode").expect("direct_mode entry exists");
    assert_eq!(direct.category, CapabilityCategory::Workflows);
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
fn suggested_questions_stays_coming_soon_until_a_producer_exists() {
    // #6464: this entry shipped at Beta with how_to "Home or Conversations >
    // Suggested prompts", sending users to look for a control that cannot
    // appear. Both surfaces that would render suggestions — the welcome chips
    // and the follow-up row — read `s.thread.suggestions`, which is filled only
    // by the `suggestions` key on the assistant-ui ExternalStoreAdapter.
    // `useOpenHumanExternalStore` does not declare it and nothing else produces
    // suggestions, so the array is permanently empty.
    //
    // This assertion is a ratchet, not a description. The precondition for
    // moving it back to Beta is a producer that actually fills that array —
    // whoever lands one changes this test in the same commit and reads this
    // comment on the way past. The cross-language tie (a Rust catalogue entry
    // versus a TypeScript render path) cannot be asserted from this crate; this
    // is the part that can.
    let cap = lookup("conversation.suggested_questions").expect("entry exists");
    assert_eq!(
        cap.status,
        CapabilityStatus::ComingSoon,
        "suggested questions must not advertise Beta while nothing fills \
         `s.thread.suggestions`"
    );
    assert!(
        !cap.how_to.contains('>'),
        "how_to must not read as a navigation breadcrumb while there is no \
         control to navigate to — it was `Home or Conversations > Suggested \
         prompts`, which is what #6464 reported"
    );
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
fn persona_pack_reports_github_mascot_manifest_destination() {
    let cap = lookup("settings.persona_pack").expect("persona pack capability exists");
    let privacy = cap.privacy.expect("persona pack is privacy-annotated");

    assert!(
        privacy.leaves_device,
        "loading the mascot library fetches a GitHub-hosted manifest"
    );
    assert_eq!(privacy.data_kind, PrivacyDataKind::Metadata);
    let haystack = privacy.destinations.join(" | ").to_lowercase();
    assert!(
        haystack.contains("github")
            && haystack.contains("raw.githubusercontent.com")
            && haystack.contains("asset"),
        "destinations must disclose the GitHub raw manifest host and manifest asset hosts, got: {:?}",
        privacy.destinations
    );
}

#[test]
fn workflow_install_discloses_the_skill_download_hosts() {
    let cap = lookup("workflows.install").expect("workflow install capability exists");
    let privacy = cap.privacy.expect("workflow install is privacy-annotated");

    assert!(
        privacy.leaves_device,
        "installing fetches SKILL.md from a remote host"
    );
    assert_eq!(privacy.data_kind, PrivacyDataKind::Metadata);
    let haystack = privacy.destinations.join(" | ").to_lowercase();
    assert!(
        haystack.contains("clawhub.ai")
            && haystack.contains("raw.githubusercontent.com")
            && haystack.contains("api.github.com"),
        "destinations must name the registry download hosts, got: {:?}",
        privacy.destinations
    );
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
        "workflows.open_connections_hub",
        "workflows.connect_google",
        "auth.backup_recovery_phrase",
        "auth.configure_tool_access",
        "settings.manage_service",
        "settings.clear_app_data",
        "local_ai.configure_provider",
        "intelligence.mcp_server",
        "intelligence.searxng_search",
        "intelligence.tool_registry",
        "intelligence.agent_library",
        "intelligence.embedding_provider_config",
        "intelligence.embedding_provider_test",
        "memory.engine",
        "memory.ask",
        "memory.learnings",
        "memory.conversations",
        "memory.documents",
        "memory.context",
        "memory.import",
        "conversation.subagent_mascots",
    ] {
        assert!(
            ids.contains(expected),
            "missing catalog capability `{expected}`"
        );
    }
}

#[test]
fn screen_intelligence_is_not_a_catalog_category_or_capability() {
    assert!(
        "screen_intelligence".parse::<CapabilityCategory>().is_err(),
        "removed category must not deserialize"
    );
    assert!(
        CapabilityCategory::ALL
            .iter()
            .all(|category| category.as_str() != "screen_intelligence"),
        "removed category must not be exposed by CapabilityCategory::ALL"
    );
    assert!(
        all_capabilities()
            .iter()
            .all(|capability| !capability.id.starts_with("screen_intelligence.")),
        "screen intelligence capability entries must be removed"
    );
}

/// The two embeddings entries surface a Settings-side configuration panel.
/// They share the same domain (`embeddings`) but are listed under the
/// Intelligence umbrella so they sit next to the memory entries / mcp_server
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

    // The settings surface they describe is the same one — make sure the
    // `how_to` strings point at it, not at an out-of-date breadcrumb.
    assert!(
        config.how_to.contains("Connections") && config.how_to.contains("Embeddings"),
        "config how_to must mention Connections → … → Embeddings, got: {}",
        config.how_to
    );
    assert!(
        test.how_to.contains("Connections") && test.how_to.contains("Embeddings"),
        "test how_to must mention Connections → … → Embeddings, got: {}",
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

/// #4884: capability `how_to` breadcrumbs are user-facing and searchable, and
/// the agent paraphrases them. Guard against the stale navigation paths that
/// sent users to menus that do not exist (`Settings > Connections`,
/// `Settings > Messaging Channels`, `Settings > Automation & Channels`). The
/// real destinations live under the top-level Connections page.
#[test]
fn catalog_how_to_uses_connections_nav_not_legacy_settings_paths() {
    const LEGACY: [&str; 6] = [
        "Settings > Connections",
        "Settings → Connections",
        "Settings > Messaging Channels",
        "Settings → Messaging Channels",
        "Settings > Automation & Channels",
        "Settings → Automation & Channels",
    ];
    for capability in all_capabilities() {
        for legacy in LEGACY {
            assert!(
                !capability.how_to.contains(legacy),
                "capability `{}` how_to still points at the removed `{legacy}` path: {}",
                capability.id,
                capability.how_to
            );
        }
    }

    // Spot-check the corrected channel + Composio breadcrumbs.
    let how_to = |id: &str| {
        all_capabilities()
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("missing capability `{id}`"))
            .how_to
    };
    assert_eq!(
        how_to("channels.connect_platform"),
        "Connections > Channels"
    );
    assert_eq!(how_to("workflows.connect_google"), "Connections > OAuth");
}

/// The `Settings > Local AI Model` panel no longer exists, and six `local_ai`
/// entries still send users to it.
///
/// `0ec68613af` removed the local-model debug panel. Its route is now a
/// redirect — `app/src/components/settings/settingsRouteElements.tsx` maps
/// `local-model-debug` to `<Navigate to="/connections?tab=llm" replace />` —
/// and no settings surface renders the string "Local AI Model" any more (the
/// only remaining occurrence in the app is a dead i18n key, `voice.openLocalAiModel`,
/// which nothing mounts).
///
/// These entries are user-visible: the Privacy panel renders whatever
/// `about_app.list` returns, so a stale breadcrumb is a user following a
/// navigation path that silently lands somewhere else. That is the same defect
/// #6464 reported for `conversation.suggested_questions`, and the guard written
/// for it (`suggested_questions_stays_coming_soon_until_a_producer_exists`,
/// above) asserts exactly this shape — it just does not cover the `local_ai`
/// domain.
///
/// This assertion is a ratchet, not a description. Two ways to satisfy it, and
/// whoever lands either reads this comment on the way past:
///
///   1. The panel comes back — then the breadcrumb is true again and naming the
///      real route satisfies the check.
///   2. The panel stays gone — then each entry either points at the surface that
///      actually serves it, or states in prose that the capability has no user
///      control yet: no breadcrumb, status unchanged, the `how_to` says where
///      the behaviour lives instead (see `local_ai.model_context_check`).
///
/// What this cannot assert: that the route named by a breadcrumb resolves in
/// the React router. That tie is cross-language and belongs to a VU or PW case.
/// This is the part that can be pinned from Rust — that no `local_ai` entry
/// names a panel title this repo no longer contains.
/// OpenHuman no longer downloads local models or installs Piper: the user
/// runs their own runtime. The catalog
/// must not advertise those capabilities.
#[test]
fn local_ai_catalog_does_not_advertise_model_downloads_or_installers() {
    for removed in [
        "local_ai.download_model",
        "local_ai.manage_model_assets",
        "local_ai.piper_installer",
    ] {
        assert!(lookup(removed).is_none(), "`{removed}` is still advertised");
    }
    let provider = lookup("local_ai.configure_provider").expect("configure_provider");
    assert!(
        provider.description.contains("ollama pull"),
        "configure_provider should tell the user to pull models themselves"
    );
}

#[test]
fn local_ai_capabilities_do_not_point_at_the_removed_local_ai_model_panel() {
    const REMOVED_PANEL: &str = "Local AI Model";

    let stale: Vec<&Capability> = all_capabilities()
        .iter()
        .filter(|capability| capability.domain == "local_ai")
        .filter(|capability| capability.how_to.contains(REMOVED_PANEL))
        .collect();

    assert!(
        stale.is_empty(),
        "{} local_ai capabilit{} advertise the removed `{REMOVED_PANEL}` panel while \
         /settings/local-model-debug redirects to /connections: {}",
        stale.len(),
        if stale.len() == 1 { "y" } else { "ies" },
        stale
            .iter()
            .map(|capability| format!("{} -> {:?}", capability.id, capability.how_to))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// The `local_ai` domain must not be empty, or the check above passes vacuously.
///
/// Written because the assertion it guards is a filter over a const table: a
/// refactor that renamed the domain, moved these entries to another catalog
/// file, or dropped them would make the stale-breadcrumb check scan zero rows
/// and report clean. A filter that matches nothing reports `ok`.
#[test]
fn local_ai_domain_is_populated_so_the_breadcrumb_check_is_not_vacuous() {
    let count = all_capabilities()
        .iter()
        .filter(|capability| capability.domain == "local_ai")
        .count();

    assert!(
        count >= 6,
        "expected the local_ai domain to carry at least the six entries the \
         breadcrumb check exists for, found {count}"
    );
}

/// Memory v2 capabilities: one per Brain chip, each pointing at the live
/// `/connections?tab=brain&brain=<chip>` surface and citing real RPC names.
#[test]
fn memory_v2_capabilities_cite_live_surface_and_rpcs() {
    for (id, chip, rpc) in [
        ("memory.engine", "engine", "memory_engine_set"),
        ("memory.ask", "ask", "memory_recall"),
        ("memory.learnings", "learnings", "memory_learn"),
        (
            "memory.conversations",
            "conversations",
            "memory_conversations_set",
        ),
        ("memory.documents", "documents", "memory_sources_add"),
        ("memory.context", "context", "memory_context_refresh"),
    ] {
        let cap = lookup(id).unwrap_or_else(|| panic!("missing `{id}`"));
        assert!(
            cap.how_to.contains(&format!("brain={chip}")),
            "{id} how_to must name its chip, got: {}",
            cap.how_to
        );
        assert!(
            cap.how_to.contains(rpc),
            "{id} how_to must cite `{rpc}`, got: {}",
            cap.how_to
        );
    }
    assert!(lookup("memory.import")
        .expect("import")
        .how_to
        .contains("memory_import_start"));
}

/// The v1 memory surface is gone; no catalog entry may advertise it.
#[test]
fn v1_memory_capabilities_are_not_advertised() {
    for id in [
        "intelligence.long_term_goals",
        "intelligence.memory_tree_retrieval",
        "intelligence.memory_pipeline_doctor",
        "intelligence.agentmemory_backend",
        "intelligence.tool_scoped_memory",
        "intelligence.memory_sync_schedule",
        "intelligence.memory_source_sync_controls",
        "intelligence.coding_session_memory",
        "intelligence.github_repo_memory_source",
        "intelligence.slack_memory_ingest",
        "intelligence.clickup_memory_ingest",
        "intelligence.remember_preferences",
    ] {
        assert!(lookup(id).is_none(), "removed capability `{id}` is back");
    }
    for c in all_capabilities() {
        let text = format!("{} {} {}", c.name, c.description, c.how_to);
        for dead in [
            "memory_tree",
            "MEMORY.md",
            "PROFILE.md",
            "memory_goals",
            "TinyCortex",
        ] {
            assert!(!text.contains(dead), "`{}` still mentions `{dead}`", c.id);
        }
    }
}
