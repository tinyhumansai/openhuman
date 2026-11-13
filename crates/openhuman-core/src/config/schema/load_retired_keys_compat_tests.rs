//! A `config.toml` written by an older build still carries settings that no
//! longer exist (issue #329). Unknown keys are ignored, not rejected: such a
//! file must load, keep every setting that still exists, and save cleanly.

use super::*;

/// Every retired key, each next to a setting that still exists so the test
/// can tell "ignored" apart from "the whole section was dropped".
const OLD_CONFIG: &str = r#"
[reliability]
provider_backoff_ms = 777
fallback_providers = ["legacy-provider"]
provider_retries = 9
model_fallbacks = { "acme/model" = ["acme/backup"] }

[agent]
agent_timeout_secs = 321
session_dual_write = false

[local_ai]
autosummary_debounce_ms = 4321
selected_tier = "ram_2_4gb"

[browser]
enabled = true
session_name = "legacy-session"
native_headless = false
native_webdriver_url = "http://127.0.0.1:9999"
native_chrome_path = "/opt/chrome"

[browser.computer_use]
endpoint = "http://127.0.0.1:8787/v1/actions"
timeout_ms = 1000
allow_remote_endpoint = true
window_allowlist = ["Terminal"]

[dashboard.event_stream]
max_entries = 42

[dashboard.diagram_viewer]
enabled = false
source_url = "http://localhost:1/x.png"
refresh_interval_seconds = 99

[security.resources]

[web_search]
max_results = 7
timeout_secs = 20

[integrations.google_places]
enabled = false

[integrations.tinyfish]
enabled = false

[integrations.parallel]
enabled = false
mode = "byo"
api_key = "enc2:00000000000000000000000000000000000000000000000000000000000000000000"
"#;

/// Dotted paths of the retired keys in [`OLD_CONFIG`].
const RETIRED_KEYS: &[&str] = &[
    "reliability.provider_retries",
    "reliability.model_fallbacks",
    "agent.session_dual_write",
    "local_ai.selected_tier",
    "browser.native_headless",
    "browser.native_webdriver_url",
    "browser.native_chrome_path",
    "browser.computer_use",
    "dashboard.diagram_viewer",
    "security.resources",
    "web_search",
    "integrations.parallel",
];

/// Loads, keeps every setting that still exists, saves, and loads again.
#[tokio::test]
async fn old_config_with_retired_keys_loads_and_saves_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("config.toml"), OLD_CONFIG).unwrap();

    let config = load_or_init_for_workspace(tmp.path()).await;
    assert_retained(&config);
    config.save().await.unwrap();

    let reloaded = load_or_init_for_workspace(tmp.path()).await;
    assert_retained(&reloaded);
}

/// The retired keys are ignored on load (recorded as unknown, never
/// decrypted, so a sealed value under one cannot fail the load) and are gone
/// from the saved file.
#[tokio::test]
async fn retired_keys_are_ignored_on_load_and_dropped_on_save() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    std::fs::write(&config_path, OLD_CONFIG).unwrap();

    let config = load_or_init_for_workspace(tmp.path()).await;
    let unknown = &config.load_diagnostics().unknown_keys;
    for retired in RETIRED_KEYS {
        assert!(
            unknown.iter().any(|key| key.starts_with(retired)),
            "{retired} should be recorded as an ignored key, got {unknown:?}"
        );
    }

    config.save().await.unwrap();
    let saved = std::fs::read_to_string(&config_path).unwrap();
    let document: toml::Value = toml::from_str(&saved).unwrap();
    for retired in RETIRED_KEYS {
        let mut node = Some(&document);
        for part in retired.split('.') {
            node = node.and_then(|value| value.get(part));
        }
        assert!(
            node.is_none(),
            "retired {retired} survived config save:\n{saved}"
        );
    }
    assert!(
        !saved.contains("enc2:0000"),
        "the sealed value under a retired key must be dropped, not kept:\n{saved}"
    );

    let reloaded = load_or_init_for_workspace(tmp.path()).await;
    assert!(
        reloaded.load_diagnostics().unknown_keys.is_empty(),
        "a saved config must not carry unknown keys: {:?}",
        reloaded.load_diagnostics().unknown_keys
    );
}

fn assert_retained(config: &Config) {
    assert_eq!(config.reliability.provider_backoff_ms, 777);
    assert_eq!(config.reliability.fallback_providers, ["legacy-provider"]);
    assert_eq!(config.agent.agent_timeout_secs, 321);
    assert_eq!(config.local_ai.autosummary_debounce_ms, 4321);
    assert!(config.browser.enabled);
    assert_eq!(
        config.browser.session_name.as_deref(),
        Some("legacy-session")
    );
    assert_eq!(config.dashboard.event_stream.max_entries, 42);
    assert!(!config.integrations.google_places.enabled);
    assert!(!config.integrations.tinyfish.enabled);
}
