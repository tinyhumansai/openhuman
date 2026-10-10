use super::*;
use crate::config::{SearchProviderSettings, SEARCH_ROLES};

fn config_with(providers: &[(&str, SearchProviderSettings)]) -> Config {
    let mut config = Config::default();
    config.search.providers = providers
        .iter()
        .map(|(name, settings)| ((*name).to_string(), *settings))
        .collect();
    config
}

fn find<'a>(resolved: &'a [ResolvedProvider], id: &str) -> &'a ResolvedProvider {
    resolved
        .iter()
        .find(|p| p.id == id)
        .expect("provider resolved")
}

#[test]
fn role_keys_match_the_contract_serde_names() {
    for role in ROLES {
        let wire = serde_json::to_value(role).unwrap();
        assert_eq!(wire.as_str(), Some(role_key(role)));
        assert_eq!(parse_role(role_key(role)), Some(role));
    }
    assert_eq!(SEARCH_ROLES.len(), ROLES.len());
    assert_eq!(parse_role("research"), None);
}

#[test]
fn managed_providers_need_a_backend_credential() {
    let config = Config::default();
    let signed_out = resolve_with(&config, false);
    assert!(!find(&signed_out, "exa").usable);
    assert_eq!(
        find(&signed_out, "exa").status(true),
        ProviderStatus::SignInRequired
    );

    let signed_in = resolve_with(&config, true);
    assert!(find(&signed_in, "exa").usable);
    assert!(find(&signed_in, "gemini").usable);
    assert_eq!(find(&signed_in, "exa").status(true), ProviderStatus::Ready);
}

#[test]
fn direct_providers_need_a_key() {
    let mut config = config_with(&[("brave", SearchProviderSettings::direct())]);
    assert_eq!(
        find(&resolve_with(&config, true), "brave").status(true),
        ProviderStatus::NeedsKey
    );
    config.search.brave.api_key = Some("k".into());
    assert!(find(&resolve_with(&config, false), "brave").usable);
}

#[test]
fn a_managed_route_on_a_byok_only_provider_resolves_direct() {
    let mut config = config_with(&[("tavily", SearchProviderSettings::managed())]);
    config.search.tavily.api_key = Some("t".into());
    let tavily = find(&resolve_with(&config, false), "tavily").clone();
    assert_eq!(tavily.route, SearchRoute::Direct);
    assert!(tavily.usable);
    assert!(!tavily.managed_capable);
}

#[test]
fn searxng_and_seltz_use_their_own_sections() {
    let mut config = config_with(&[
        ("searxng", SearchProviderSettings::direct()),
        ("seltz", SearchProviderSettings::direct()),
    ]);
    config.seltz.api_key = Some("s".into());
    let resolved = resolve_with(&config, false);
    assert!(find(&resolved, "searxng").usable);
    assert!(find(&resolved, "seltz").usable);
}

#[test]
fn search_off_makes_nothing_usable() {
    let mut config = Config::default();
    config.search.enabled = Some(false);
    let resolved = resolve_with(&config, true);
    assert!(resolved.iter().all(|p| !p.usable));
    assert_eq!(
        find(&resolved, "exa").status(false),
        ProviderStatus::SearchOff
    );
}

#[test]
fn unselected_providers_are_disabled() {
    let config = Config::default();
    let brave = find(&resolve_with(&config, true), "brave").clone();
    assert!(!brave.enabled);
    assert_eq!(brave.status(true), ProviderStatus::Disabled);
}

#[test]
fn effective_roles_follow_configured_order_and_skip_unusable_providers() {
    let mut config = config_with(&[
        ("exa", SearchProviderSettings::managed()),
        ("gemini", SearchProviderSettings::managed()),
        ("brave", SearchProviderSettings::direct()),
    ]);
    config.search.brave.api_key = Some("b".into());
    config
        .search
        .roles
        .insert("search".into(), vec!["brave".into(), "exa".into()]);
    let signed_in = resolve_with(&config, true);
    assert_eq!(
        effective_role_providers(&signed_in, &config, Role::Search),
        vec!["brave".to_string(), "exa".to_string()]
    );
    let signed_out = resolve_with(&config, false);
    assert_eq!(
        effective_role_providers(&signed_out, &config, Role::Search),
        vec!["brave".to_string()]
    );
    assert!(effective_role_providers(&signed_out, &config, Role::Answer).is_empty());
}

#[test]
fn role_order_drops_providers_that_cannot_serve_the_role() {
    let mut config = Config::default();
    config
        .search
        .roles
        .insert("answer".into(), vec!["brave".into(), "gemini".into()]);
    assert_eq!(
        role_order(&config, Role::Answer),
        vec!["gemini".to_string()]
    );
}

#[test]
fn an_empty_role_uses_the_module_default_order() {
    let config = Config::default();
    let order = role_order(&config, Role::Search);
    assert_eq!(order.first().map(String::as_str), Some("exa"));
}

#[test]
fn keenable_is_usable_without_a_key_once_selected() {
    let mut config = config_with(&[("keenable", SearchProviderSettings::direct())]);
    let resolved = resolve_with(&config, false);
    let keenable = find(&resolved, "keenable").clone();
    assert!(keenable.usable);
    assert!(keenable.key_optional);
    assert!(!keenable.key_configured);
    assert_eq!(keenable.status(true), ProviderStatus::Ready);
    assert_eq!(
        effective_role_providers(&resolved, &config, Role::Search),
        vec!["keenable".to_string()]
    );
    assert_eq!(
        effective_role_providers(&resolved, &config, Role::Contents),
        vec!["keenable".to_string()]
    );

    config.search.keenable.api_key = Some("k".into());
    assert!(find(&resolve_with(&config, false), "keenable").key_configured);
}

#[test]
fn keenable_stays_off_until_selected_and_is_the_only_key_optional_provider() {
    let resolved = resolve_with(&Config::default(), true);
    let keenable = find(&resolved, "keenable").clone();
    assert!(!keenable.enabled);
    assert!(!keenable.usable);
    assert_eq!(keenable.status(true), ProviderStatus::Disabled);
    let optional: Vec<&str> = resolved
        .iter()
        .filter(|p| p.key_optional)
        .map(|p| p.id)
        .collect();
    assert_eq!(optional, vec!["keenable"]);
}
