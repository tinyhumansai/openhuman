use super::*;

// ── [components] migration ───────────────────────────────────────────

fn parse(raw: &str) -> Config {
    toml::from_str(raw).expect("config parses")
}

fn migrated(raw: &str, tinymemes: Option<&str>) -> Config {
    let mut c = parse(raw);
    crate::config::schema::load::migrate::migrate_components_with(&mut c, raw, tinymemes);
    c
}

#[test]
fn old_config_with_every_legacy_flag_off_disables_each_component() {
    let raw = r#"
[hosting]
enabled = false
[search]
enabled = false
[mcp_client]
enabled = false
[memory]
engine = "none"
[composio]
enabled = true
"#;
    let c = migrated(raw, Some("off"));
    assert!(!c.components.hosting);
    assert!(!c.components.search);
    assert!(!c.components.mcp);
    assert!(!c.components.memory);
    assert!(!c.components.memes);
    assert_eq!(c.memory.engine, "none", "engine is kept");
    assert!(c.components.flows && c.components.voice, "others stay on");
}

#[test]
fn hosting_enabled_false_alone_maps_to_components_hosting() {
    let c = migrated("[hosting]\nenabled = false\n", None);
    assert!(!c.components.hosting);
    assert!(c.components.search && c.components.mcp && c.components.memory);
}

#[test]
fn enabled_or_absent_legacy_flags_change_nothing() {
    let c = migrated(
        "[hosting]\nenabled = true\n[mcp_client]\nenabled = true\n",
        Some("on"),
    );
    assert_eq!(
        c.components,
        crate::config::schema::ComponentsConfig::default()
    );
    let c = migrated("", None);
    assert_eq!(
        c.components,
        crate::config::schema::ComponentsConfig::default()
    );
}

#[test]
fn explicit_components_key_beats_a_legacy_flag() {
    let raw = "[components]\nhosting = true\n[hosting]\nenabled = false\n";
    let c = migrated(raw, None);
    assert!(c.components.hosting);
}

#[test]
fn migration_round_trip_is_idempotent() {
    let raw = "[hosting]\nenabled = false\n[mcp_client]\nenabled = false\n";
    let first = migrated(raw, None);
    let saved = toml::to_string(&first).unwrap();
    // Re-load the saved text: `components` is now explicit, legacy keys inert.
    let second = migrated(&saved, None);
    assert_eq!(first.components, second.components);
    let resaved = toml::to_string(&second).unwrap();
    assert_eq!(saved, resaved);
}

#[test]
fn user_turning_a_component_back_on_survives_the_stale_legacy_key() {
    let first = migrated("[hosting]\nenabled = false\n", None);
    let mut saved: toml::Value = toml::from_str(&toml::to_string(&first).unwrap()).unwrap();
    saved["components"]["hosting"] = toml::Value::Boolean(true);
    let text = toml::to_string(&saved).unwrap();
    assert!(migrated(&text, None).components.hosting);
}

#[test]
fn composio_enabled_is_accepted_and_dropped() {
    let c = parse("[composio]\nenabled = true\nmode = \"direct\"\n");
    assert_eq!(c.composio.mode, "direct");
    let out = toml::to_string(&c).unwrap();
    let v: toml::Value = toml::from_str(&out).unwrap();
    assert!(v["composio"].get("enabled").is_none());
}

#[test]
fn legacy_tokenjuice_router_off_maps_to_components_juice() {
    let c = migrated("[tokenjuice]\nrouter_enabled = false\n", None);
    assert!(!c.components.juice);
    assert!(c.components.search && c.components.mcp && c.components.memory);
}

#[test]
fn tokenjuice_router_on_or_absent_keeps_juice_on() {
    assert!(
        migrated("[tokenjuice]\nrouter_enabled = true\n", None)
            .components
            .juice
    );
    assert!(
        migrated("[tokenjuice]\nccr_enabled = false\n", None)
            .components
            .juice
    );
}

#[test]
fn explicit_components_juice_beats_legacy_router_flag() {
    let raw = "[components]\njuice = true\n[tokenjuice]\nrouter_enabled = false\n";
    assert!(migrated(raw, None).components.juice);
}

#[test]
fn juice_migration_round_trip_is_idempotent() {
    let first = migrated("[tokenjuice]\nrouter_enabled = false\n", None);
    let saved = toml::to_string(&first).unwrap();
    let second = migrated(&saved, None);
    assert!(!second.components.juice);
    assert_eq!(saved, toml::to_string(&second).unwrap());
}

#[test]
fn voice_switches_all_explicitly_off_disable_the_voice_component() {
    let c = migrated(
        "[dictation]\nenabled = false\n[voice_server]\nauto_start = false\nalways_on_enabled = false\n",
        None,
    );
    assert!(!c.components.voice);
    assert!(c.components.flows, "others stay on");
}

#[test]
fn voice_stays_on_when_any_legacy_voice_switch_is_on_or_unset() {
    let c = migrated(
        "[dictation]\nenabled = true\n[voice_server]\nauto_start = false\nalways_on_enabled = false\n",
        None,
    );
    assert!(c.components.voice);
    let c = migrated("[voice_server]\nauto_start = false\n", None);
    assert!(c.components.voice, "unset keys are not an opt-out");
}

#[test]
fn explicit_components_voice_wins_over_legacy_voice_switches() {
    let c = migrated(
        "[components]\nvoice = true\n[dictation]\nenabled = false\n[voice_server]\nauto_start = false\nalways_on_enabled = false\n",
        None,
    );
    assert!(c.components.voice);
}

#[test]
fn composio_mode_disabled_migrates_to_components_connectors_off() {
    let c = migrated("[composio]\nmode = \"disabled\"\n", None);
    assert!(!c.components.connectors);
    assert_eq!(
        c.composio.mode, "disabled",
        "mode stays as the route selector"
    );
    assert!(c.components.flows && c.components.search);
}

#[test]
fn composio_backend_mode_or_explicit_components_key_keeps_connectors_on() {
    assert!(
        migrated("[composio]\nmode = \"direct\"\n", None)
            .components
            .connectors
    );
    let c = migrated(
        "[composio]\nmode = \"disabled\"\n[components]\nconnectors = true\n",
        None,
    );
    assert!(c.components.connectors, "explicit components key wins");
}

#[test]
fn sandbox_enabled_false_maps_to_components_sandbox() {
    let c = migrated("[sandbox]\nenabled = false\n", None);
    assert!(!c.components.sandbox);
    assert_eq!(c.sandbox.enabled, Some(false), "old key kept");
    let c = migrated("[security.sandbox]\nenabled = false\n", None);
    assert!(!c.components.sandbox);
}

#[test]
fn sandbox_auto_or_on_or_explicit_component_keeps_the_component_on() {
    for raw in [
        "[sandbox]\nenabled = true\n",
        "[sandbox]\nbackend = \"auto\"\n",
        "[sandbox]\nenabled = false\n[components]\nsandbox = true\n",
    ] {
        assert!(migrated(raw, None).components.sandbox, "{raw}");
    }
}
