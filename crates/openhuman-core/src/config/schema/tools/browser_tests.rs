use super::*;

fn with_unattended(actions: &[&str]) -> BrowserConfig {
    BrowserConfig {
        unattended_actions: actions.iter().map(|action| (*action).to_owned()).collect(),
        ..BrowserConfig::default()
    }
}

#[test]
fn unattended_actions_default_to_none() {
    let config = BrowserConfig::default();
    assert!(config.unattended_actions.is_empty());
    for kind in UNATTENDED_BROWSER_ACTIONS {
        assert!(!config.allows_unattended(kind), "{kind}");
    }
}

#[test]
fn unattended_actions_parse_from_toml_and_default_when_absent() {
    let parsed: BrowserConfig =
        toml::from_str(r#"unattended_actions = ["click", "press"]"#).unwrap();
    assert_eq!(parsed.unattended_actions, vec!["click", "press"]);
    let absent: BrowserConfig = toml::from_str("enabled = true").unwrap();
    assert!(absent.unattended_actions.is_empty());
}

#[test]
fn only_listed_known_kinds_are_allowed_unattended() {
    let config = with_unattended(&["click", " Press ", "task_step"]);
    assert!(config.allows_unattended("click"));
    assert!(config.allows_unattended("press"));
    assert!(config.allows_unattended("task_step"));
    assert!(!config.allows_unattended("fill"));
    assert!(!config.allows_unattended("type"));
}

#[test]
fn unknown_unattended_names_are_reported_and_never_allow_anything() {
    let config = with_unattended(&["click", "navigate", "hover", "*", ""]);
    assert_eq!(
        config.unknown_unattended_actions(),
        vec!["navigate", "hover", "*", ""]
    );
    assert!(!config.allows_unattended("navigate"));
    assert!(!config.allows_unattended("hover"));
    assert!(!config.allows_unattended("*"));
    assert!(with_unattended(&["double_click", "select", "check"])
        .unknown_unattended_actions()
        .is_empty());
}

#[test]
fn known_unattended_names_match_the_gated_action_wire_names() {
    assert_eq!(
        UNATTENDED_BROWSER_ACTIONS,
        &[
            "click",
            "double_click",
            "fill",
            "type",
            "press",
            "select",
            "check",
            "task_step"
        ]
    );
}

#[test]
fn the_requested_kind_is_normalized_like_the_configured_entries() {
    let config = with_unattended(&["click"]);
    assert!(config.allows_unattended(" CLICK "));
    assert!(config.allows_unattended("Click"));
}

#[test]
fn an_allowed_kind_resolves_to_its_static_canonical_name() {
    let config = with_unattended(&["Press"]);
    let canonical: Option<&'static str> = config.unattended_kind(" PRESS ");
    assert_eq!(canonical, Some("press"));
    assert_eq!(config.unattended_kind("click"), None);
    assert_eq!(config.unattended_kind("#secret selector"), None);
}

#[test]
fn upstream_proxies_parse_from_toml_and_default_to_none() {
    let parsed: BrowserConfig = toml::from_str(
        r#"upstream_proxies = [{ domains = ["reddit.com", "redd.it"], url = "http://user:hunter2@gate.example:7000" }]"#,
    )
    .unwrap();
    assert_eq!(
        parsed.upstream_proxies,
        vec![BrowserUpstreamProxy {
            domains: vec!["reddit.com".into(), "redd.it".into()],
            url: "http://user:hunter2@gate.example:7000".into(),
        }]
    );
    let absent: BrowserConfig = toml::from_str("enabled = true").unwrap();
    assert!(absent.upstream_proxies.is_empty());
    // Unset, the key is not written back.
    let written = toml::to_string(&BrowserConfig::default()).unwrap();
    assert!(!written.contains("upstream_proxies"));
}

#[test]
fn an_upstream_proxy_url_never_appears_in_debug_output() {
    let config = BrowserConfig {
        upstream_proxies: vec![BrowserUpstreamProxy {
            domains: vec!["reddit.com".into()],
            url: "http://user:hunter2@gate.example:7000".into(),
        }],
        ..BrowserConfig::default()
    };
    let debug = format!("{config:?}");
    assert!(!debug.contains("hunter2"));
    assert!(!debug.contains("gate.example"));
    assert!(debug.contains("reddit.com"));
}
