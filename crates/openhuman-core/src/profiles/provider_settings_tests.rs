use super::*;
use crate::config::{PresentationMode, SearchEngineCredentials, SearchProviderSettings};
use tinysearch_bus::Role;

fn deployment(exa: Option<&str>, gemini: Option<&str>) -> crate::core::runtime::SaasConfig {
    let mut deployment = crate::core::runtime::SaasConfig::new("/srv/fixture");
    let mut search = crate::config::SearchConfig::default();
    search.providers = std::collections::BTreeMap::from([
        ("exa".into(), SearchProviderSettings::direct()),
        ("gemini".into(), SearchProviderSettings::direct()),
    ]);
    search.roles = std::collections::BTreeMap::from([
        ("search".into(), vec!["exa".into(), "gemini".into()]),
        ("answer".into(), vec!["gemini".into()]),
        ("contents".into(), vec!["exa".into()]),
    ]);
    search.exa = SearchEngineCredentials {
        api_key: exa.map(str::to_owned),
        base_url: Some("http://127.0.0.1:1234".into()),
    };
    search.gemini = SearchEngineCredentials {
        api_key: gemini.map(str::to_owned),
        base_url: Some("http://127.0.0.1:2345".into()),
    };
    deployment.search = Some(search);
    deployment.search_role_names = std::collections::BTreeMap::from([
        (Role::Search, "web_search".into()),
        (Role::Answer, "web_answer".into()),
        (Role::Contents, "web_contents".into()),
    ]);
    deployment
}

#[test]
fn direct_search_binding_never_serializes_operator_credentials() {
    let deployment = deployment(Some("operator-exa-key"), Some("operator-gemini-key"));
    let settings = ProfileProviderSettings::from_saas(&deployment).unwrap();
    let mut config = crate::config::Config::default();
    settings.bind(&mut config);
    let serialized = toml::to_string(&config).unwrap();
    let debug = format!("{config:?} {deployment:?} {settings:?}");
    for secret in ["operator-exa-key", "operator-gemini-key"] {
        assert!(!serialized.contains(secret));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn role_aliases_resolve_once_and_canonical_names_cannot_bypass_presentation() {
    let settings =
        ProfileProviderSettings::from_saas(&deployment(Some("key"), Some("key"))).unwrap();
    assert_eq!(
        settings.canonical_role_name("web_search"),
        Some("web_search_tool")
    );
    assert_eq!(
        settings.canonical_role_name("web_answer"),
        Some("web_answer_tool")
    );
    assert_eq!(
        settings.canonical_role_name("web_contents"),
        Some("web_contents_tool")
    );
    for name in [
        "web_search_tool",
        "web_answer_tool",
        "web_contents_tool",
        "shell",
        "exa_search",
        "unknown",
        "web_search_tool_tool",
    ] {
        assert_eq!(settings.canonical_role_name(name), None, "{name}");
    }
}

#[test]
fn rejects_collisions_duplicates_unsafe_identifiers_and_alias_chains() {
    for alias in [
        "",
        " ",
        "shell",
        "memory",
        "show_page",
        "web_answer_tool",
        "exa_search",
        "bad.name",
        "chain->web_search",
        "web_answer",
    ] {
        let mut deployment = deployment(Some("key"), Some("key"));
        deployment
            .search_role_names
            .insert(Role::Search, alias.into());
        assert!(
            ProfileProviderSettings::from_saas(&deployment).is_err(),
            "{alias}"
        );
    }
}

#[test]
fn rejects_managed_routes_extra_providers_and_provider_native_presentation() {
    let mut managed = deployment(Some("key"), None);
    managed
        .search
        .as_mut()
        .unwrap()
        .providers
        .insert("exa".into(), SearchProviderSettings::managed());
    assert!(ProfileProviderSettings::from_saas(&managed).is_err());
    let mut extra = deployment(Some("key"), None);
    extra
        .search
        .as_mut()
        .unwrap()
        .providers
        .insert("brave".into(), SearchProviderSettings::direct());
    assert!(ProfileProviderSettings::from_saas(&extra).is_err());
    let mut native = deployment(Some("key"), None);
    native.search.as_mut().unwrap().presentation = PresentationMode::AllTools;
    assert!(ProfileProviderSettings::from_saas(&native).is_err());
}

#[cfg(feature = "modules")]
#[test]
fn private_module_configuration_preserves_direct_urls_and_never_resolves_managed_routes() {
    let deployment = deployment(Some("exa-fixture-key"), Some("gemini-fixture-key"));
    let settings = ProfileProviderSettings::from_saas(&deployment).unwrap();
    let mut config = crate::config::Config::default();
    config.api_key = Some("ambient-managed-key-must-not-be-read".into());
    settings.bind(&mut config);
    let module = crate::modules::search::module_config(&config);
    assert_eq!(
        module
            .providers
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["exa", "gemini"]
    );
    assert_eq!(
        module.providers["exa"].base_url.as_deref(),
        Some("http://127.0.0.1:1234")
    );
    assert_eq!(
        module.providers["gemini"].base_url.as_deref(),
        Some("http://127.0.0.1:2345")
    );
    assert_eq!(
        module.providers["exa"].credential.as_deref(),
        Some("exa-fixture-key")
    );
    assert_eq!(
        module.providers["gemini"].credential.as_deref(),
        Some("gemini-fixture-key")
    );
    assert!(module.backend.credential.is_none());
    assert!(module.backend.base_url.is_none());
    assert_eq!(module.presentation.roles[&Role::Search], ["exa", "gemini"]);
    assert_eq!(module.presentation.roles[&Role::Answer], ["gemini"]);
    assert_eq!(module.presentation.roles[&Role::Contents], ["exa"]);
    assert_eq!(
        crate::modules::search::configured_tool_specs(&config)
            .iter()
            .map(|spec| spec.name.as_str())
            .collect::<Vec<_>>(),
        ["web_search", "web_answer", "web_contents"]
    );
}

#[cfg(feature = "modules")]
#[test]
fn blank_search_keys_remove_tools_instead_of_inheriting_managed_defaults() {
    for (exa, gemini, expected) in [
        (None, None, vec![]),
        (Some(" "), Some("\t"), vec![]),
        (Some("key"), None, vec!["web_search", "web_contents"]),
        (None, Some("key"), vec!["web_search", "web_answer"]),
    ] {
        let settings = ProfileProviderSettings::from_saas(&deployment(exa, gemini)).unwrap();
        let mut config = crate::config::Config::default();
        settings.bind(&mut config);
        let module = crate::modules::search::module_config(&config);
        assert_eq!(module.enabled, !expected.is_empty());
        assert_eq!(
            crate::modules::search::configured_tool_specs(&config)
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(module.backend.base_url.is_none());
        assert!(module.backend.credential.is_none());
    }
}
