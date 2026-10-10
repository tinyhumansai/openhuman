use super::*;
use crate::config::SearchProviderSettings;

fn config_in(dir: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.config_path = dir.join("config.toml");
    config.workspace_dir = dir.join("workspace");
    config.secrets.encrypt = false;
    config
}

fn sign_in(config: &Config) {
    crate::security::credentials::api_key::store_api_key(config, "backend-sentinel").unwrap();
}

fn select(config: &mut Config, providers: &[(&str, SearchProviderSettings)]) {
    config.search.providers = providers
        .iter()
        .map(|(name, settings)| ((*name).to_string(), *settings))
        .collect();
}

#[test]
fn signed_in_defaults_route_exa_and_gemini_through_the_backend() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    sign_in(&config);
    let payload = module_config(&config);
    for name in ["exa", "gemini"] {
        let provider = &payload.providers[name];
        assert!(provider.enabled, "{name}");
        assert_eq!(provider.route, ProviderRoute::Backend, "{name}");
        assert!(provider.credential.is_none(), "{name}");
    }
    assert_eq!(payload.backend.auth_mode, BackendAuthMode::ApiKey);
    assert_eq!(
        payload.backend.credential.as_deref(),
        Some("backend-sentinel")
    );
    assert!(!format!("{payload:?}").contains("backend-sentinel"));
    assert_eq!(payload.presentation.mode, PresentationMode::Roles);
    assert!(!payload.providers.contains_key("parallel"));
}

#[test]
fn signed_out_managed_providers_are_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    let payload = module_config(&config);
    assert!(!payload.providers["exa"].enabled);
    assert!(!payload.providers["gemini"].enabled);
    assert!(payload.backend.credential.is_none());
    assert!(configured_tool_specs(&config).is_empty());
}

#[test]
fn direct_provider_config_keeps_limits_and_private_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    select(&mut config, &[("tavily", SearchProviderSettings::direct())]);
    config.search.tavily.api_key = Some("test-secret".into());
    config.search.max_results = 12;
    config.search.timeout_secs = 22;
    let payload = module_config(&config);
    let tavily = &payload.providers["tavily"];
    assert!(tavily.enabled);
    assert_eq!(tavily.route, ProviderRoute::Direct);
    assert_eq!(tavily.credential.as_deref(), Some("test-secret"));
    assert_eq!(tavily.max_results, Some(12));
    assert_eq!(tavily.timeout_secs, Some(22));
    assert!(!format!("{payload:?}").contains("test-secret"));
}

#[test]
fn role_tools_follow_provider_selection() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    sign_in(&config);
    let names: Vec<String> = configured_tool_specs(&config)
        .into_iter()
        .map(|spec| spec.name)
        .collect();
    assert_eq!(
        names,
        vec![
            "web_search_tool".to_string(),
            "web_answer_tool".to_string(),
            "web_contents_tool".to_string()
        ]
    );
    select(
        &mut config,
        &[("gemini", SearchProviderSettings::managed())],
    );
    let names: Vec<String> = configured_tool_specs(&config)
        .into_iter()
        .map(|spec| spec.name)
        .collect();
    assert_eq!(names, vec!["web_answer_tool".to_string()]);
}

#[test]
fn roles_are_passed_in_configured_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config
        .search
        .roles
        .insert("search".into(), vec!["brave".into(), "exa".into()]);
    let payload = module_config(&config);
    assert_eq!(
        payload.presentation.roles[&tinysearch_bus::Role::Search],
        vec!["brave".to_string(), "exa".to_string()]
    );
    assert!(payload
        .presentation
        .roles
        .contains_key(&tinysearch_bus::Role::Answer));
}

#[test]
fn all_tools_presentation_maps_through() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.search.presentation = SearchPresentation::OneProvider;
    config.search.presentation_provider = Some("exa".into());
    let payload = module_config(&config);
    assert_eq!(payload.presentation.mode, PresentationMode::OneProvider);
    assert_eq!(payload.presentation.provider.as_deref(), Some("exa"));
}

#[test]
fn gemini_deep_research_uses_the_direct_key_beside_managed_gemini() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    sign_in(&config);
    config.search.gemini.api_key = Some("test-secret".into());
    let payload = module_config(&config);
    assert_eq!(payload.providers["gemini"].route, ProviderRoute::Backend);
    let deep = &payload.providers["gemini_deep_research"];
    assert!(deep.enabled);
    assert_eq!(deep.route, ProviderRoute::Direct);
    assert_eq!(deep.credential.as_deref(), Some("test-secret"));
}

#[test]
fn gemini_deep_research_is_off_without_a_key() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    sign_in(&config);
    assert!(!module_config(&config).providers["gemini_deep_research"].enabled);
}

#[test]
fn seltz_and_searxng_use_their_own_sections() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    select(
        &mut config,
        &[
            ("seltz", SearchProviderSettings::direct()),
            ("searxng", SearchProviderSettings::direct()),
        ],
    );
    config.seltz.api_key = Some("test-secret".into());
    config.searxng.base_url = "http://127.0.0.1:8888".into();
    let payload = module_config(&config);
    assert!(payload.providers["seltz"].enabled);
    assert_eq!(
        payload.providers["seltz"].credential.as_deref(),
        Some("test-secret")
    );
    assert!(payload.providers["searxng"].enabled);
    assert_eq!(
        payload.providers["searxng"].base_url.as_deref(),
        Some("http://127.0.0.1:8888")
    );
}

#[test]
fn search_off_disables_the_module_and_every_provider() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    sign_in(&config);
    config.search.enabled = Some(false);
    let payload = module_config(&config);
    assert!(!payload.enabled);
    assert!(payload.providers.values().all(|provider| !provider.enabled));
    assert!(configured_tool_specs(&config).is_empty());
}

#[test]
fn keenable_is_offered_without_a_key_and_gets_one_when_stored() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    select(
        &mut config,
        &[("keenable", SearchProviderSettings::direct())],
    );
    let payload = module_config(&config);
    let keenable = &payload.providers["keenable"];
    assert!(keenable.enabled);
    assert_eq!(keenable.route, ProviderRoute::Direct);
    assert!(keenable.credential.is_none());
    let names: Vec<String> = configured_tool_specs(&config)
        .into_iter()
        .map(|spec| spec.name)
        .collect();
    assert_eq!(
        names,
        vec![
            "web_search_tool".to_string(),
            "web_contents_tool".to_string()
        ]
    );

    config.search.keenable.api_key = Some("test-secret".into());
    let payload = module_config(&config);
    assert_eq!(
        payload.providers["keenable"].credential.as_deref(),
        Some("test-secret")
    );
    assert!(!format!("{payload:?}").contains("test-secret"));
}
