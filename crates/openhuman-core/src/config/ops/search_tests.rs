use super::*;
use crate::config::SearchProviderSettings;

fn patch(value: Value) -> SearchSettingsPatch {
    serde_json::from_value(value).expect("patch deserializes")
}

fn provider<'a>(view: &'a Value, id: &str) -> &'a Value {
    view["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap_or_else(|| panic!("{id} listed"))
}

#[test]
fn enabling_a_byok_provider_and_saving_its_key() {
    let mut config = Config::default();
    apply_search_patch(
        &mut config,
        patch(json!({"providers": {"brave": {"enabled": true, "api_key": " k "}}})),
    )
    .unwrap();
    assert_eq!(
        config.search.providers.get("brave"),
        Some(&SearchProviderSettings::direct())
    );
    assert_eq!(config.search.brave.key(), Some("k"));

    apply_search_patch(
        &mut config,
        patch(json!({"providers": {"brave": {"api_key": ""}}})),
    )
    .unwrap();
    assert!(config.search.brave.key().is_none());
}

#[test]
fn switching_exa_to_its_own_key() {
    let mut config = Config::default();
    apply_search_patch(
        &mut config,
        patch(json!({"providers": {"exa": {"route": "direct", "api_key": "e"}}})),
    )
    .unwrap();
    assert_eq!(config.search.route("exa"), SearchRoute::Direct);
    assert_eq!(config.search.exa.key(), Some("e"));
}

#[test]
fn routes_a_provider_cannot_take_are_rejected() {
    let mut config = Config::default();
    let err = apply_search_patch(
        &mut config,
        patch(json!({"providers": {"brave": {"route": "managed"}}})),
    )
    .unwrap_err();
    assert!(err.contains("does not support the managed route"), "{err}");
    // TinyFish is own-key only: the backend never proxied it.
    let err = apply_search_patch(
        &mut config,
        patch(json!({"providers": {"tinyfish": {"route": "managed"}}})),
    )
    .unwrap_err();
    assert!(err.contains("does not support the managed route"), "{err}");
    assert!(apply_search_patch(
        &mut config,
        patch(json!({"providers": {"bing": {"enabled": true}}}))
    )
    .unwrap_err()
    .contains("unknown search provider"));
    let err = apply_search_patch(
        &mut config,
        patch(json!({"providers": {"parallel": {"route": "managed"}}})),
    )
    .unwrap_err();
    assert!(err.contains("does not support the managed route"), "{err}");
    assert!(apply_search_patch(
        &mut config,
        patch(json!({"providers": {"searxng": {"api_key": "x"}}}))
    )
    .is_err());
}

#[test]
fn roles_are_validated_deduplicated_and_resettable() {
    let mut config = Config::default();
    apply_search_patch(
        &mut config,
        patch(json!({"roles": {"search": ["Brave", "exa", "brave"]}})),
    )
    .unwrap();
    assert_eq!(
        config.search.roles.get("search"),
        Some(&vec!["brave".to_string(), "exa".to_string()])
    );
    let err = apply_search_patch(&mut config, patch(json!({"roles": {"answer": ["brave"]}})))
        .unwrap_err();
    assert!(err.contains("cannot serve the answer role"), "{err}");
    assert!(apply_search_patch(&mut config, patch(json!({"roles": {"research": []}}))).is_err());
    apply_search_patch(&mut config, patch(json!({"roles": {"search": []}}))).unwrap();
    assert!(!config.search.roles.contains_key("search"));
}

#[test]
fn legacy_engine_from_an_older_client_still_works() {
    let mut config = Config::default();
    apply_search_patch(&mut config, patch(json!({"engine": "disabled"}))).unwrap();
    assert!(!config.search.is_enabled());
    apply_search_patch(&mut config, patch(json!({"engine": "managed"}))).unwrap();
    assert!(config.search.is_enabled());
    apply_search_patch(&mut config, patch(json!({"engine": "parallel"}))).unwrap();
    assert_eq!(config.search.route("parallel"), SearchRoute::Direct);
    assert!(apply_search_patch(&mut config, patch(json!({"engine": "bing"}))).is_err());
}

#[test]
fn limits_presentation_and_allowlist_validate() {
    let mut config = Config::default();
    assert!(apply_search_patch(&mut config, patch(json!({"max_results": 0}))).is_err());
    assert!(apply_search_patch(&mut config, patch(json!({"timeout_secs": 500}))).is_err());
    assert!(apply_search_patch(&mut config, patch(json!({"presentation": "grid"}))).is_err());
    apply_search_patch(
        &mut config,
        patch(json!({"presentation": "all_tools", "max_results": 7,
                     "allowed_domains": ["b.com", " a.com ", "b.com"], "allow_all": false})),
    )
    .unwrap();
    assert_eq!(config.search.presentation, SearchPresentation::AllTools);
    assert_eq!(config.search.max_results, 7);
    assert_eq!(
        config.http_request.allowed_domains,
        vec!["a.com".to_string(), "b.com".to_string()]
    );
}

#[test]
fn seltz_and_searxng_patches_update_their_sections() {
    let mut config = Config::default();
    apply_search_patch(
        &mut config,
        patch(json!({"providers": {
            "seltz": {"enabled": true, "api_key": "s"},
            "searxng": {"enabled": true, "base_url": "https://sx.example"}
        }})),
    )
    .unwrap();
    assert!(config.seltz.enabled);
    assert_eq!(config.seltz.api_key.as_deref(), Some("s"));
    assert!(config.searxng.enabled);
    assert_eq!(config.searxng.base_url, "https://sx.example");
    assert!(apply_search_patch(
        &mut config,
        patch(json!({"providers": {"searxng": {"base_url": "ftp://x"}}}))
    )
    .is_err());
}

#[test]
fn settings_view_reports_status_roles_and_never_keys() {
    let mut config = Config::default();
    config.search.gemini.api_key = Some("secret-gemini".into());
    let view = search_settings_json_with(&config, true);
    assert_eq!(view["enabled"], true);
    assert_eq!(view["presentation"], "roles");
    let exa = provider(&view, "exa");
    assert_eq!(exa["status"], "ready");
    assert_eq!(exa["route"], "managed");
    assert_eq!(exa["routes"], json!(["managed", "direct"]));
    let gemini = provider(&view, "gemini");
    assert_eq!(gemini["deep_research_available"], true);
    let tinyfish = provider(&view, "tinyfish");
    assert_eq!(tinyfish["routes"], json!(["direct"]));
    assert_eq!(tinyfish["takes_key"], true);
    assert_eq!(tinyfish["docs_url"], "https://agent.tinyfish.ai/api-keys");
    assert_eq!(provider(&view, "brave")["status"], "disabled");
    // Deep research rides on the Gemini key and is reported on the gemini row.
    assert_eq!(view["effective_roles"]["answer"], json!(["gemini", "exa"]));
    assert_eq!(view["effective_roles"]["search"], json!(["exa"]));
    assert!(!view.to_string().contains("secret-gemini"));
    assert!(view["providers"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p["id"] != "gemini_deep_research"));

    let signed_out = search_settings_json_with(&config, false);
    assert_eq!(provider(&signed_out, "exa")["status"], "sign_in_required");
    assert_eq!(signed_out["effective_roles"]["search"], json!([]));
}

#[test]
fn tinyfish_takes_its_own_key() {
    let mut config = Config::default();
    apply_search_patch(
        &mut config,
        patch(json!({"providers": {"tinyfish": {"enabled": true, "api_key": " tf-key "}}})),
    )
    .unwrap();
    assert_eq!(config.search.tinyfish.key(), Some("tf-key"));
    let view = search_settings_json_with(&config, true);
    let tinyfish = provider(&view, "tinyfish");
    assert_eq!(tinyfish["key_configured"], true);
    assert_eq!(tinyfish["route"], "direct");
    assert!(!view.to_string().contains("tf-key"));
}

#[test]
fn keenable_turns_on_without_a_key_and_takes_an_optional_one() {
    let mut config = Config::default();
    apply_search_patch(
        &mut config,
        patch(json!({"providers": {"keenable": {"enabled": true}}})),
    )
    .unwrap();
    assert_eq!(
        config.search.providers.get("keenable"),
        Some(&SearchProviderSettings::direct())
    );
    let view = search_settings_json_with(&config, false);
    let keenable = provider(&view, "keenable");
    assert_eq!(keenable["label"], "Keenable");
    assert_eq!(keenable["routes"], json!(["direct"]));
    assert_eq!(keenable["takes_key"], true);
    assert_eq!(keenable["key_optional"], true);
    assert_eq!(keenable["key_configured"], false);
    assert_eq!(keenable["status"], "ready");
    assert_eq!(keenable["docs_url"], "https://keenable.ai/console");
    assert_eq!(provider(&view, "brave")["key_optional"], false);
    assert_eq!(view["effective_roles"]["search"], json!(["keenable"]));
    assert_eq!(view["effective_roles"]["contents"], json!(["keenable"]));

    apply_search_patch(
        &mut config,
        patch(json!({"providers": {"keenable": {"api_key": " kn-key "}}})),
    )
    .unwrap();
    assert_eq!(config.search.keenable.key(), Some("kn-key"));
    let view = search_settings_json_with(&config, false);
    assert_eq!(provider(&view, "keenable")["key_configured"], true);
    assert!(!view.to_string().contains("kn-key"));

    let err = apply_search_patch(
        &mut config,
        patch(json!({"providers": {"keenable": {"route": "managed"}}})),
    )
    .unwrap_err();
    assert!(err.contains("does not support the managed route"), "{err}");
}
