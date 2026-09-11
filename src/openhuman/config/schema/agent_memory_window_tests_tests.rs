use super::*;

#[test]
fn presets_are_strictly_ordered_and_bounded() {
    let m = MemoryContextWindow::Minimal.limits();
    let b = MemoryContextWindow::Balanced.limits();
    let e = MemoryContextWindow::Extended.limits();
    let max = MemoryContextWindow::Maximum.limits();

    // Recall cap grows monotonically with preset size.
    assert!(m.max_memory_context_chars < b.max_memory_context_chars);
    assert!(b.max_memory_context_chars < e.max_memory_context_chars);
    assert!(e.max_memory_context_chars < max.max_memory_context_chars);

    // Tree summary caps grow monotonically too.
    assert!(m.per_namespace_max_chars < b.per_namespace_max_chars);
    assert!(b.per_namespace_max_chars < e.per_namespace_max_chars);
    assert!(e.per_namespace_max_chars < max.per_namespace_max_chars);
    assert!(m.total_tree_max_chars < max.total_tree_max_chars);

    // Hard ceiling is bounded — Maximum still leaves headroom in a
    // typical 200k-token context window.
    assert!(max.total_tree_max_chars <= 128_000);
}

#[test]
fn balanced_matches_legacy_defaults() {
    // Balanced preset must keep historical behaviour: 2 000 char
    // recall budget and 32 000 char total tree-summary cap (used to
    // be hard-coded constants in `agent/prompts/types.rs`).
    let b = MemoryContextWindow::Balanced.limits();
    assert_eq!(b.max_memory_context_chars, 2_000);
    assert_eq!(b.per_namespace_max_chars, 8_000);
    assert_eq!(b.total_tree_max_chars, 32_000);
}

#[test]
fn default_agent_config_is_unmigrated_and_resolves_to_balanced_caps() {
    // Default = `memory_window: None` (unmigrated). The recall cap
    // falls back to the legacy `max_memory_context_chars` default
    // (2 000), which matches Balanced — so the resolved limits are
    // byte-identical to the historical behaviour.
    let cfg = AgentConfig::default();
    assert_eq!(cfg.memory_window, None);
    assert_eq!(
        cfg.resolved_memory_limits(),
        MemoryContextWindow::Balanced.limits()
    );
}

#[test]
fn explicit_preset_is_authoritative_and_ignores_legacy_raw_field() {
    // Once Minimal is chosen, the preset's recall cap (800) is what
    // the harness sees — even if the legacy raw field still holds a
    // wider value from before the user picked a preset. Without
    // this, switching to `Minimal` in the UI would silently fail to
    // shrink the recall budget.
    let cfg = AgentConfig {
        memory_window: Some(MemoryContextWindow::Minimal),
        max_memory_context_chars: 4_000,
        ..AgentConfig::default()
    };
    assert_eq!(
        cfg.resolved_memory_limits(),
        MemoryContextWindow::Minimal.limits(),
        "explicit preset must override legacy raw field"
    );
}

#[test]
fn unmigrated_config_honours_legacy_raw_field_within_safety_ceiling() {
    // Unmigrated power-user config with a legacy override of 4 000
    // keeps that recall cap on upgrade so behaviour doesn't shrink
    // silently. Tree caps come from the Balanced baseline because
    // older builds had no per-namespace cap on this code path.
    let cfg = AgentConfig {
        memory_window: None,
        max_memory_context_chars: 4_000,
        ..AgentConfig::default()
    };
    let limits = cfg.resolved_memory_limits();
    assert_eq!(limits.max_memory_context_chars, 4_000);
    assert_eq!(
        limits.per_namespace_max_chars,
        MemoryContextWindow::Balanced
            .limits()
            .per_namespace_max_chars
    );

    // An unbounded legacy value is clamped to the Maximum preset's
    // recall cap so on-disk overrides can't blow up prompts.
    let runaway = AgentConfig {
        memory_window: None,
        max_memory_context_chars: 1_000_000,
        ..AgentConfig::default()
    };
    assert_eq!(
        runaway.resolved_memory_limits().max_memory_context_chars,
        MemoryContextWindow::Maximum
            .limits()
            .max_memory_context_chars
    );
}

#[test]
fn switching_preset_can_shrink_recall_below_legacy_value() {
    // Regression for the CodeRabbit concern: an unmigrated config
    // with a wide legacy override that then explicitly picks
    // `Minimal` in the UI must end up with the Minimal recall cap,
    // not the legacy value.
    let mut cfg = AgentConfig {
        memory_window: None,
        max_memory_context_chars: 4_000,
        ..AgentConfig::default()
    };
    assert_eq!(cfg.resolved_memory_limits().max_memory_context_chars, 4_000);
    cfg.memory_window = Some(MemoryContextWindow::Minimal);
    assert_eq!(
        cfg.resolved_memory_limits().max_memory_context_chars,
        MemoryContextWindow::Minimal
            .limits()
            .max_memory_context_chars
    );
}

#[test]
fn from_str_opt_round_trips() {
    for window in [
        MemoryContextWindow::Minimal,
        MemoryContextWindow::Balanced,
        MemoryContextWindow::Extended,
        MemoryContextWindow::Maximum,
    ] {
        assert_eq!(
            MemoryContextWindow::from_str_opt(window.as_str()),
            Some(window)
        );
    }
    assert_eq!(
        MemoryContextWindow::from_str_opt("MAXIMUM"),
        Some(MemoryContextWindow::Maximum)
    );
    assert_eq!(MemoryContextWindow::from_str_opt("nonsense"), None);
}

#[test]
fn enum_serializes_as_lowercase_string() {
    let json = serde_json::to_string(&MemoryContextWindow::Extended).unwrap();
    assert_eq!(json, "\"extended\"");
    let back: MemoryContextWindow = serde_json::from_str("\"minimal\"").unwrap();
    assert_eq!(back, MemoryContextWindow::Minimal);
}

#[test]
fn empty_channel_permissions_with_existing_channels_migrates_to_execute() {
    // Legacy install: channel_permissions empty but the user has two
    // channels configured. The migration seeds web + each existing
    // channel = execute so the new fail-closed default doesn't
    // regress them.
    let mut cfg = AgentConfig::default();
    assert!(cfg.channel_permissions.is_empty());

    let known = vec!["telegram".to_string(), "discord".to_string()];
    let migrated = cfg.migrate_channel_permissions_if_legacy(known.iter());

    assert!(migrated, "legacy install must migrate");
    assert_eq!(cfg.channel_permissions.len(), 3);
    for ch in ["web", "telegram", "discord"] {
        assert_eq!(
            cfg.channel_permissions.get(ch).map(String::as_str),
            Some("execute"),
            "expected execute for channel {ch}"
        );
    }
}

#[test]
fn migrate_channel_permissions_idempotent() {
    let mut cfg = AgentConfig::default();
    cfg.migrate_channel_permissions_if_legacy(vec!["telegram".to_string()].iter());
    let again = cfg.migrate_channel_permissions_if_legacy(vec!["telegram".to_string()].iter());
    assert!(!again, "second migration call must be a no-op");
}

#[test]
fn migrate_channel_permissions_with_no_channels_is_noop() {
    // Fresh install with no configured channels and an empty map —
    // no migration needed (the engine fails closed on lookup).
    let mut cfg = AgentConfig::default();
    let migrated = cfg.migrate_channel_permissions_if_legacy(Vec::<String>::new());
    assert!(!migrated);
    assert!(cfg.channel_permissions.is_empty());
}

#[test]
fn agents_md_enabled_defaults_to_true() {
    assert!(
        AgentConfig::default().agents_md_enabled,
        "AGENTS.md loading must be on by default"
    );
}

#[test]
fn agents_md_enabled_defaults_true_when_field_omitted() {
    // A config that predates the field must deserialize with the feature on
    // (matches the `#[serde(default = ...)]` contract).
    let cfg: AgentConfig = serde_json::from_str("{}").unwrap();
    assert!(cfg.agents_md_enabled);
}

#[test]
fn agents_md_enabled_roundtrips_when_disabled() {
    let cfg: AgentConfig = serde_json::from_str(r#"{"agents_md_enabled": false}"#).unwrap();
    assert!(!cfg.agents_md_enabled);
}
