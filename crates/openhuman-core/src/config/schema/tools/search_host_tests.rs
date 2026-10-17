use super::*;
use crate::config::ProviderRoute;

const KEY: &str = "pinned-brave-secret";

fn pinned() -> SearchConfig {
    let mut search = SearchConfig {
        enabled: Some(false),
        ..SearchConfig::default()
    };
    search
        .pin_host_credential(
            "brave",
            SearchHostCredential::new(format!(" {KEY} ")).with_base_url("http://127.0.0.1:9"),
        )
        .expect("brave takes a key");
    search
}

#[test]
fn pinning_turns_search_on_and_routes_the_provider_direct() {
    let search = pinned();
    assert!(search.is_host_pinned());
    assert_eq!(search.enabled, Some(true));
    assert_eq!(search.providers["brave"], SearchProviderSettings::direct());
    assert_eq!(search.route("brave"), ProviderRoute::Direct);
    assert_eq!(search.direct_key("brave"), Some(KEY));
    assert_eq!(
        search.host_credential("brave").and_then(|c| c.base_url()),
        Some("http://127.0.0.1:9")
    );
}

#[test]
fn an_unpinned_config_is_not_host_pinned_and_reads_its_stored_key() {
    let mut search = SearchConfig::default();
    assert!(!search.is_host_pinned());
    search.tavily.api_key = Some("stored".into());
    assert_eq!(search.direct_key("tavily"), Some("stored"));
    assert_eq!(search.direct_key("brave"), None);
}

#[test]
fn a_pinned_key_wins_over_the_stored_one() {
    let mut search = pinned();
    search.brave.api_key = Some("stored".into());
    assert_eq!(search.direct_key("brave"), Some(KEY));
}

#[test]
fn deep_research_uses_the_pinned_gemini_key() {
    let mut search = SearchConfig::default();
    search
        .pin_host_credential("gemini", SearchHostCredential::new("g-key"))
        .unwrap();
    assert_eq!(search.direct_key("gemini_deep_research"), Some("g-key"));
}

#[test]
fn rejects_a_provider_that_takes_no_key() {
    let mut search = SearchConfig::default();
    for provider in ["searxng", "seltz", "gemini_deep_research", "nope"] {
        let error = search
            .pin_host_credential(provider, SearchHostCredential::new("k"))
            .expect_err(provider);
        assert_eq!(error, UnknownKeyedSearchProvider(provider.into()));
        assert_eq!(
            error.to_string(),
            format!("search provider '{provider}' does not take an API key")
        );
    }
    assert!(!search.is_host_pinned());
}

#[test]
fn an_empty_pinned_key_is_no_key() {
    let mut search = SearchConfig::default();
    search
        .pin_host_credential("exa", SearchHostCredential::new("  ").with_base_url(" "))
        .unwrap();
    assert_eq!(search.direct_key("exa"), None);
    assert_eq!(search.host_credential("exa").unwrap().base_url(), None);
}

#[test]
fn a_pinned_key_is_never_serialized_or_printed() {
    let search = pinned();
    let json = serde_json::to_string(&search).unwrap();
    let toml = toml::to_string(&search).unwrap();
    let debug = format!("{search:?}");
    for rendered in [&json, &toml, &debug] {
        assert!(!rendered.contains(KEY), "{rendered}");
    }
    assert!(debug.contains("<redacted>"));
    // A reload from what was written carries no pin.
    let reloaded: SearchConfig = toml::from_str(&toml).unwrap();
    assert!(!reloaded.is_host_pinned());
}

#[test]
fn existing_config_toml_search_section_deserializes_and_round_trips_unchanged() {
    let existing = r#"
presentation = "one_provider"
presentation_provider = "exa"

[providers.exa]
enabled = true
route = "managed"

[providers.brave]
enabled = true
route = "direct"
"#;
    let search: SearchConfig = toml::from_str(existing).unwrap();
    assert_eq!(
        search.presentation,
        crate::config::PresentationMode::OneProvider
    );
    assert_eq!(search.route("exa"), ProviderRoute::Backend);
    assert_eq!(search.route("brave"), ProviderRoute::Direct);

    // The config spelling stays `managed` on write, never the wire `backend`.
    let written = toml::to_string(&search).unwrap();
    assert!(written.contains("route = \"managed\""), "{written}");
    assert!(!written.contains("backend"), "{written}");
    assert!(
        written.contains("presentation = \"one_provider\""),
        "{written}"
    );
}
