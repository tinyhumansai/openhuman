use super::*;
use crate::openhuman::memory::sync::composio::SyncTarget;

fn sync_target(toolkit: &str, connection_id: &str) -> SyncTarget {
    SyncTarget {
        toolkit: toolkit.to_string(),
        connection_id: connection_id.to_string(),
    }
}

#[test]
fn build_upsert_targets_formats_label_and_preserves_order() {
    let targets = vec![
        sync_target("gmail", "ca_WaktIDFlZwXO"),
        sync_target("slack", "short"),
    ];
    let out = build_upsert_targets(&targets);
    assert_eq!(out.len(), 2);
    // (toolkit, connection_id, label) — toolkit/connection_id carried through verbatim.
    assert_eq!(out[0].0, "gmail");
    assert_eq!(out[0].1, "ca_WaktIDFlZwXO");
    assert_eq!(out[0].2, "Gmail · IDFlZwXO");
    assert_eq!(out[1].0, "slack");
    assert_eq!(out[1].1, "short");
    assert_eq!(out[1].2, "Slack · short");
}

#[test]
fn build_upsert_targets_empty_is_empty() {
    let out = build_upsert_targets(&[]);
    assert!(out.is_empty());
}

#[test]
fn short_id_truncates_ascii() {
    assert_eq!(short_id("ca_WaktIDFlZwXO"), "IDFlZwXO");
}

#[test]
fn short_id_short_input_passthrough() {
    assert_eq!(short_id("abc"), "abc");
    assert_eq!(short_id("12345678"), "12345678");
}

#[test]
fn short_id_utf8_safe() {
    // Multi-byte chars would have panicked with byte-slicing.
    let s = "🦀🐢🐙🦊🐼🐰🐯🐸🦁";
    let out = short_id(s);
    assert_eq!(out.chars().count(), 8);
}

// ── Caps migration ──────────────────────────────────────────────────────────
//
// The transform came home with `apply_composio_source_caps_migration` (#5560),
// so its predicate is this crate's to pin. These exercise
// `apply_caps_defaults_to_entries` — the real production function, not a
// re-statement of it — because the migration's whole risk is in which entries
// it decides to touch: an over-eager pass overwrites a cap the user chose, and
// a shy one leaves an enabled, cap-less connector syncing at the provider's
// internal ceiling.

fn composio_entry(
    id: &str,
    toolkit: &str,
    enabled: bool,
    max_items: Option<u32>,
    sync_depth_days: Option<u32>,
) -> MemorySourceEntry {
    MemorySourceEntry {
        id: id.to_string(),
        kind: SourceKind::Composio,
        label: toolkit.to_string(),
        enabled,
        toolkit: Some(toolkit.to_string()),
        connection_id: Some(format!("conn_{id}")),
        path: None,
        glob: None,
        url: None,
        branch: None,
        paths: Vec::new(),
        max_commits: None,
        max_issues: None,
        max_prs: None,
        query: None,
        since_days: None,
        max_items,
        selector: None,
        max_tokens_per_sync: None,
        max_cost_per_sync_usd: None,
        sync_depth_days,
    }
}

#[test]
fn migration_flips_disabled_capless_entry_to_enabled_with_caps() {
    let mut sources = vec![composio_entry("s1", "gmail", false, None, None)];
    let count = apply_caps_defaults_to_entries(&mut sources);
    assert_eq!(count, 1);
    assert!(sources[0].enabled);
    assert_eq!(sources[0].max_items, Some(100));
    assert_eq!(sources[0].sync_depth_days, Some(30));
}

#[test]
fn migration_applies_defaults_to_enabled_capless_entry() {
    // An already-enabled but cap-less source must also receive defaults —
    // otherwise its first sync runs at the provider's large internal ceiling.
    let mut sources = vec![composio_entry("s2", "slack", true, None, None)];
    let count = apply_caps_defaults_to_entries(&mut sources);
    assert_eq!(count, 1);
    assert!(sources[0].enabled);
    assert_eq!(sources[0].max_items, Some(50));
    assert_eq!(sources[0].sync_depth_days, Some(14));
}

#[test]
fn migration_leaves_user_customised_caps_untouched() {
    // The user set max_items explicitly, so the migration must not override it
    // — and must not flip `enabled` on its way past either.
    let mut sources = vec![composio_entry("s3", "notion", false, Some(5), None)];
    let count = apply_caps_defaults_to_entries(&mut sources);
    assert_eq!(count, 0, "entry with user-set caps must not be migrated");
    assert!(!sources[0].enabled, "enabled must not be flipped");
    assert_eq!(sources[0].max_items, Some(5), "user cap must be preserved");
}

#[test]
fn migration_is_noop_on_empty_list() {
    let mut sources: Vec<MemorySourceEntry> = vec![];
    assert_eq!(apply_caps_defaults_to_entries(&mut sources), 0);
}

#[test]
fn migration_applies_correct_defaults_per_toolkit() {
    let toolkits = [
        ("gmail", Some(100u32), Some(30u32)),
        ("slack", Some(50), Some(14)),
        ("notion", Some(30), Some(30)),
        ("linear", Some(50), Some(30)),
        ("clickup", Some(50), Some(30)),
        ("github", Some(50), Some(30)),
        ("unknown", Some(30), Some(14)),
    ];
    for (toolkit, exp_items, exp_days) in &toolkits {
        let mut sources = vec![composio_entry("sid", toolkit, false, None, None)];
        apply_caps_defaults_to_entries(&mut sources);
        assert_eq!(
            sources[0].max_items, *exp_items,
            "max_items mismatch for toolkit={toolkit}"
        );
        assert_eq!(
            sources[0].sync_depth_days, *exp_days,
            "sync_depth_days mismatch for toolkit={toolkit}"
        );
    }
}

/// The non-Composio arm goes through `apply_kind_defaults`, which fills the
/// kind's own caps and — unlike the Composio arm — never enables anything and
/// never counts towards the returned tally.
#[test]
fn migration_fills_kind_defaults_without_counting_them() {
    let mut repo = composio_entry("s4", "github", false, None, None);
    repo.kind = SourceKind::GithubRepo;
    repo.toolkit = None;
    repo.connection_id = None;
    repo.url = Some("https://github.com/tinyhumansai/openhuman".to_string());

    let mut sources = vec![repo];
    let count = apply_caps_defaults_to_entries(&mut sources);

    assert_eq!(count, 0, "only composio entries count as migrated");
    assert!(
        !sources[0].enabled,
        "kind defaults must not enable a source"
    );
    assert_eq!(sources[0].max_prs, Some(10));
    assert!(sources[0].max_issues.is_some());
}
