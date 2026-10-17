use super::*;
use crate::config::schema::tools::http::HttpRequestConfig;

fn legacy(toml_body: &str) -> SearchConfig {
    toml::from_str(toml_body).expect("legacy search section parses")
}

fn provider(cfg: &SearchConfig, name: &str) -> Option<SearchProviderSettings> {
    cfg.providers.get(name).copied()
}

#[test]
fn fresh_defaults_are_managed_exa_and_gemini_with_roles_presentation() {
    let cfg = SearchConfig::default();
    assert!(!cfg.needs_migration());
    assert!(cfg.is_enabled());
    assert_eq!(cfg.presentation, PresentationMode::Roles);
    assert_eq!(
        provider(&cfg, "exa"),
        Some(SearchProviderSettings::managed())
    );
    assert_eq!(
        provider(&cfg, "gemini"),
        Some(SearchProviderSettings::managed())
    );
    assert_eq!(cfg.providers.len(), 2);
    assert!(cfg.roles.is_empty());
}

#[test]
fn legacy_omitted_presentation_migrates_to_routed_roles() {
    let mut cfg = legacy("engine = \"managed\"\n");
    cfg.migrate_legacy(LegacySearchInputs::default());
    assert_eq!(cfg.presentation, PresentationMode::Roles);
    assert_eq!(cfg.schema_version, SEARCH_SCHEMA_VERSION);
}

#[test]
fn legacy_all_tools_migrates_to_routed_roles() {
    let mut cfg = legacy("engine = \"managed\"\npresentation = \"all_tools\"\n");
    cfg.migrate_legacy(LegacySearchInputs::default());
    assert_eq!(cfg.presentation, PresentationMode::Roles);
}

// v2 files were forced onto `all_tools`, which hides the routed
// `web_*_tool`s that agent tool scopes allowlist (no web search at all).
#[test]
fn v2_all_tools_moves_back_to_roles_once() {
    let mut cfg = legacy("schema_version = 2\npresentation = \"all_tools\"\n");
    assert!(cfg.needs_migration());
    assert!(cfg.migrate_legacy(LegacySearchInputs::default()));
    assert_eq!(cfg.presentation, PresentationMode::Roles);
    assert_eq!(cfg.schema_version, SEARCH_SCHEMA_VERSION);
    assert!(!cfg.migrate_legacy(LegacySearchInputs::default()));
}

#[test]
fn v2_migration_keeps_providers_roles_and_other_presentations() {
    let mut cfg = legacy(
        "schema_version = 2\npresentation = \"router\"\n\
         [providers.brave]\nenabled = true\nroute = \"direct\"\n\
         [roles]\nsearch = [\"brave\", \"exa\"]\n",
    );
    let before_providers = cfg.providers.clone();
    let before_roles = cfg.roles.clone();
    assert!(cfg.migrate_legacy(LegacySearchInputs::default()));
    assert_eq!(cfg.presentation, PresentationMode::Router);
    assert_eq!(cfg.providers, before_providers);
    assert_eq!(cfg.roles, before_roles);
    assert!(provider(&cfg, "brave").is_some());
}

#[test]
fn a_current_all_tools_choice_is_kept() {
    let mut cfg = legacy(&format!(
        "schema_version = {SEARCH_SCHEMA_VERSION}\npresentation = \"all_tools\"\n"
    ));
    assert!(!cfg.migrate_legacy(LegacySearchInputs::default()));
    assert_eq!(cfg.presentation, PresentationMode::AllTools);
}

#[test]
fn http_request_defaults_to_allow_all() {
    // Web research works out of the box: the default allowlist is the
    // wildcard. The SSRF guard (url_guard) still blocks local/private
    // hosts regardless, so this only opens public sites.
    let cfg = HttpRequestConfig::default();
    assert_eq!(cfg.allowed_domains, vec!["*".to_string()]);
    assert_eq!(cfg.max_response_size, 1_000_000);
    assert_eq!(cfg.timeout_secs, 30);
}

#[test]
fn a_file_without_schema_version_needs_migration() {
    let cfg = legacy("engine = \"managed\"\n");
    assert!(cfg.needs_migration());
    assert!(cfg.providers.is_empty());
}

#[test]
fn legacy_managed_becomes_managed_exa_and_gemini() {
    let mut cfg = legacy("engine = \"managed\"\n");
    assert!(cfg.migrate_legacy(LegacySearchInputs::default()));
    assert!(cfg.is_enabled());
    assert_eq!(
        provider(&cfg, "exa"),
        Some(SearchProviderSettings::managed())
    );
    assert_eq!(
        provider(&cfg, "gemini"),
        Some(SearchProviderSettings::managed())
    );
    assert!(cfg.roles.is_empty());
    assert_eq!(cfg.schema_version, SEARCH_SCHEMA_VERSION);
    assert!(cfg.engine.is_none());
}

#[test]
fn migration_is_idempotent() {
    let mut cfg = legacy("engine = \"brave\"\n[brave]\napi_key = \"k\"\n");
    assert!(cfg.migrate_legacy(LegacySearchInputs::default()));
    let snapshot = format!("{cfg:?}");
    assert!(!cfg.migrate_legacy(LegacySearchInputs {
        tinyfish_active: true,
        ..Default::default()
    }));
    assert_eq!(format!("{cfg:?}"), snapshot);
}

#[test]
fn legacy_disabled_stays_disabled_but_keeps_providers_configured() {
    let mut cfg = legacy("engine = \"disabled\"\n");
    cfg.migrate_legacy(LegacySearchInputs::default());
    assert!(!cfg.is_enabled());
    assert!(cfg.providers.contains_key("exa"));
}

#[test]
fn legacy_byok_engine_leads_the_search_role() {
    let mut cfg = legacy("engine = \"tavily\"\n[tavily]\napi_key = \"t\"\n");
    cfg.migrate_legacy(LegacySearchInputs::default());
    assert_eq!(
        provider(&cfg, "tavily"),
        Some(SearchProviderSettings::direct())
    );
    assert_eq!(
        cfg.roles.get(SEARCH_ROLE_SEARCH),
        Some(&vec!["tavily".to_string(), "exa".to_string()])
    );
}

#[test]
fn legacy_byok_engine_without_key_does_not_claim_a_role() {
    let mut cfg = legacy("engine = \"brave\"\n");
    cfg.migrate_legacy(LegacySearchInputs::default());
    assert!(!cfg.providers.contains_key("brave"));
    assert!(cfg.roles.is_empty());
}

#[test]
fn legacy_exa_engine_with_key_keeps_exa_direct() {
    let mut cfg = legacy("engine = \"exa\"\n[exa]\napi_key = \"e\"\n");
    cfg.migrate_legacy(LegacySearchInputs::default());
    assert_eq!(
        provider(&cfg, "exa"),
        Some(SearchProviderSettings::direct())
    );
    assert_eq!(cfg.exa.key(), Some("e"));
}

#[test]
fn exa_key_under_managed_engine_stays_managed() {
    let mut cfg = legacy("engine = \"managed\"\n[exa]\napi_key = \"e\"\n");
    cfg.migrate_legacy(LegacySearchInputs::default());
    assert_eq!(
        provider(&cfg, "exa"),
        Some(SearchProviderSettings::managed())
    );
}

#[test]
fn a_parallel_key_is_kept_as_a_direct_provider() {
    let mut cfg = legacy("engine = \"parallel\"\n[parallel]\napi_key = \"p\"\n");
    cfg.migrate_legacy(LegacySearchInputs::default());
    assert_eq!(
        provider(&cfg, "parallel"),
        Some(SearchProviderSettings::direct())
    );
    assert_eq!(cfg.parallel.key(), Some("p"));
    assert_eq!(
        cfg.roles.get(SEARCH_ROLE_SEARCH),
        Some(&vec!["parallel".to_string(), "exa".to_string()])
    );
    let written = toml::to_string(&cfg).unwrap();
    assert!(!written.contains("engine"), "{written}");
    assert!(!written.contains("parallel_route"), "{written}");
}

#[test]
fn managed_parallel_without_a_key_is_dropped() {
    let mut cfg =
        legacy("enabled_providers = [\"managed\", \"parallel\"]\nparallel_route = \"backend\"\n");
    cfg.migrate_legacy(LegacySearchInputs::default());
    assert!(!cfg.providers.contains_key("parallel"));
    assert_eq!(
        provider(&cfg, "exa"),
        Some(SearchProviderSettings::managed())
    );
}

#[test]
fn parallel_is_direct_only() {
    let mut cfg = SearchConfig::default();
    cfg.providers
        .insert("parallel".into(), SearchProviderSettings::managed());
    assert_eq!(cfg.route("parallel"), ProviderRoute::Direct);
}

#[test]
fn explicit_provider_selection_maps_managed_and_routes() {
    let mut cfg = legacy(
        "enabled = true\nenabled_providers = [\"managed\", \"parallel\", \"gemini\", \"tinyfish\", \"querit\"]\n\
         parallel_route = \"backend\"\ngemini_route = \"direct\"\n[gemini]\napi_key = \"g\"\n",
    );
    cfg.migrate_legacy(LegacySearchInputs::default());
    assert_eq!(
        provider(&cfg, "exa"),
        Some(SearchProviderSettings::managed())
    );
    assert_eq!(
        provider(&cfg, "gemini"),
        Some(SearchProviderSettings::direct())
    );
    // TinyFish is own-key only; without a key it migrates switched off.
    assert_eq!(
        provider(&cfg, "tinyfish"),
        Some(SearchProviderSettings {
            enabled: false,
            route: ProviderRoute::Direct,
        })
    );
    assert_eq!(
        provider(&cfg, "querit"),
        Some(SearchProviderSettings::direct())
    );
    assert!(!cfg.providers.contains_key("parallel"));
    assert!(cfg.enabled_providers.is_none());
}

#[test]
fn legacy_toggles_outside_search_migrate_when_active() {
    let mut cfg = legacy("engine = \"managed\"\nenabled_providers = [\"managed\", \"brave\"]\n");
    cfg.migrate_legacy(LegacySearchInputs {
        tinyfish_active: true,
        seltz_active: true,
        searxng_active: true,
        tinyfish_api_key: Some("tf-key".into()),
    });
    // The legacy integration's key carries over, so TinyFish stays on (direct).
    assert_eq!(
        provider(&cfg, "tinyfish"),
        Some(SearchProviderSettings::direct())
    );
    assert_eq!(cfg.tinyfish.key(), Some("tf-key"));
    assert_eq!(
        provider(&cfg, "seltz"),
        Some(SearchProviderSettings::direct())
    );
    assert_eq!(
        provider(&cfg, "searxng"),
        Some(SearchProviderSettings::direct())
    );
}

#[test]
fn selected_legacy_exa_keeps_own_key_when_managed_is_also_selected() {
    let mut cfg =
        legacy("enabled_providers = [\"exa\", \"managed\"]\n[exa]\napi_key = \"exa-key\"\n");
    cfg.migrate_legacy(LegacySearchInputs::default());
    assert_eq!(
        provider(&cfg, "exa"),
        Some(SearchProviderSettings::direct())
    );
}

#[test]
fn managed_presentation_provider_is_cleared() {
    let mut cfg = legacy("presentation = \"router\"\npresentation_provider = \"managed\"\n");
    cfg.migrate_legacy(LegacySearchInputs::default());
    assert_eq!(cfg.presentation, PresentationMode::Router);
    assert!(cfg.presentation_provider.is_none());
}

#[test]
fn route_is_direct_for_providers_that_cannot_be_managed() {
    let mut cfg = SearchConfig::default();
    cfg.providers
        .insert("brave".into(), SearchProviderSettings::managed());
    assert_eq!(cfg.route("brave"), ProviderRoute::Direct);
    assert_eq!(cfg.route("exa"), ProviderRoute::Backend);
    assert_eq!(cfg.route("unknown"), ProviderRoute::Direct);
}

#[test]
fn enabled_provider_names_skip_disabled_entries() {
    let mut cfg = SearchConfig::default();
    cfg.providers.insert(
        "tavily".into(),
        SearchProviderSettings {
            enabled: false,
            route: ProviderRoute::Direct,
        },
    );
    let names: Vec<_> = cfg.enabled_provider_names().into_iter().collect();
    assert_eq!(names, vec!["exa".to_string(), "gemini".to_string()]);
}

#[test]
fn route_and_presentation_parse_accept_legacy_spellings() {
    assert_eq!(
        ProviderRoute::parse("backend"),
        Some(ProviderRoute::Backend)
    );
    assert_eq!(
        ProviderRoute::parse(" Direct "),
        Some(ProviderRoute::Direct)
    );
    assert_eq!(ProviderRoute::parse("other"), None);
    assert_eq!(
        PresentationMode::parse("all_tools"),
        Some(PresentationMode::AllTools)
    );
    assert_eq!(PresentationMode::parse("nope"), None);
}

#[test]
fn credentials_resolve_per_provider() {
    let mut cfg = SearchConfig::default();
    cfg.credentials_mut("gemini").unwrap().api_key = Some(" g ".into());
    assert_eq!(
        cfg.credentials("gemini_deep_research").unwrap().key(),
        Some("g")
    );
    assert!(cfg.credentials("seltz").is_none());
}

#[test]
fn v2_managed_tinyfish_without_a_key_is_turned_off() {
    let mut cfg =
        legacy("schema_version = 2\n[providers.tinyfish]\nenabled = true\nroute = \"managed\"\n");
    assert!(cfg.migrate_legacy(LegacySearchInputs::default()));
    assert_eq!(
        provider(&cfg, "tinyfish"),
        Some(SearchProviderSettings {
            enabled: false,
            route: ProviderRoute::Direct,
        })
    );
}

#[test]
fn v2_tinyfish_with_a_key_stays_on_and_goes_direct() {
    let mut cfg = legacy(
        "schema_version = 2\n[providers.tinyfish]\nenabled = true\nroute = \"managed\"\n\
         [tinyfish]\napi_key = \"tf\"\n",
    );
    assert!(cfg.migrate_legacy(LegacySearchInputs::default()));
    assert_eq!(
        provider(&cfg, "tinyfish"),
        Some(SearchProviderSettings::direct())
    );
}

#[test]
fn tinyfish_takes_its_own_key_and_is_never_managed() {
    let mut cfg = SearchConfig::default();
    assert!(cfg.credentials_mut("tinyfish").is_some());
    cfg.providers
        .insert("tinyfish".into(), SearchProviderSettings::managed());
    assert_eq!(cfg.route("tinyfish"), ProviderRoute::Direct);
}

#[test]
fn debug_never_discloses_direct_search_credentials() {
    let mut config = SearchConfig::default();
    config.exa.api_key = Some("u2-unique-search-secret".into());
    assert!(!format!("{config:?}").contains("u2-unique-search-secret"));
}
