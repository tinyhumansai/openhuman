//! Tests for the surrounding module.

use super::*;

/// A folder source, the shape the prefix tests start from.
fn folder_entry(id: &str) -> MemorySourceEntry {
    MemorySourceEntry {
        id: id.into(),
        kind: SourceKind::Folder,
        label: "x".into(),
        enabled: true,
        toolkit: None,
        connection_id: None,
        path: Some("/tmp".into()),
        glob: None,
        url: None,
        branch: None,
        paths: Vec::new(),
        query: None,
        since_days: None,
        max_items: None,
        max_commits: None,
        max_issues: None,
        max_prs: None,
        selector: None,
        max_tokens_per_sync: None,
        max_cost_per_sync_usd: None,
        sync_depth_days: None,
    }
}

/// The four branches of the prefix scheme, ported from the engine's own test.
///
/// The one deliberate difference from that test is the missing `%`: the driver
/// places its own wildcard now, so what crosses is a literal prefix. Everything
/// left of it — including the trailing separator — is unchanged, which is what
/// keeps `mem_src:src_a:` from also counting `mem_src:src_ab:`.
#[test]
fn source_id_prefix_dispatch() {
    let mut entry = folder_entry("src_abc");
    assert_eq!(source_id_prefix(&entry), "mem_src:src_abc:");

    // A Composio source is matched on its connection, not just its toolkit: a
    // second Gmail account must not count the first's chunks.
    entry.kind = SourceKind::Composio;
    entry.toolkit = Some("gmail".into());
    entry.connection_id = Some("conn-1".into());
    assert_eq!(source_id_prefix(&entry), "gmail:conn-1:");

    // Connection-less entries must not widen to the bare toolkit prefix --
    // that matched every gmail connection's chunks, so a malformed source
    // reported another connection's counts as its own.
    entry.connection_id = None;
    assert_eq!(source_id_prefix(&entry), "gmail:__no_connection__:");

    entry.toolkit = None;
    assert_eq!(source_id_prefix(&entry), "__no_toolkit__:");
}

/// The thresholds, at and either side of each boundary.
///
/// `<=` on both, so 30 000 ms is still `Active` and 300 000 ms is still
/// `Recent` — the engine's own comparison, kept because a panel that flips a
/// label one millisecond earlier than it used to is a behaviour change nobody
/// asked for.
#[test]
fn freshness_thresholds_match_the_engine() {
    let now = 1_000_000_000_i64;
    assert_eq!(FreshnessLabel::from_age_ms(None, now), FreshnessLabel::Idle);
    assert_eq!(
        FreshnessLabel::from_age_ms(Some(now), now),
        FreshnessLabel::Active
    );
    assert_eq!(
        FreshnessLabel::from_age_ms(Some(now - 30_000), now),
        FreshnessLabel::Active
    );
    assert_eq!(
        FreshnessLabel::from_age_ms(Some(now - 30_001), now),
        FreshnessLabel::Recent
    );
    assert_eq!(
        FreshnessLabel::from_age_ms(Some(now - 300_000), now),
        FreshnessLabel::Recent
    );
    assert_eq!(
        FreshnessLabel::from_age_ms(Some(now - 300_001), now),
        FreshnessLabel::Idle
    );
    // A chunk stamped in the future (a skewed remote clock on an imported
    // item) reads as fresh rather than overflowing.
    assert_eq!(
        FreshnessLabel::from_age_ms(Some(now + 60_000), now),
        FreshnessLabel::Active
    );
}

/// The serde spelling the memory-sources panel matches on. It is `snake_case`
/// in the engine's copy and has to stay `snake_case` here, because this rides a
/// response whose shape is otherwise unchanged by the move.
#[test]
fn freshness_serialises_snake_case() {
    assert_eq!(
        serde_json::to_string(&FreshnessLabel::Active).unwrap(),
        "\"active\""
    );
    assert_eq!(
        serde_json::to_string(&FreshnessLabel::Recent).unwrap(),
        "\"recent\""
    );
    assert_eq!(
        serde_json::to_string(&FreshnessLabel::Idle).unwrap(),
        "\"idle\""
    );
}

/// The five fields, their names, and the fact that `last_chunk_at_ms` is
/// carried as `null` rather than omitted — the engine's `SourceStatus` had no
/// `skip_serializing_if`, and a field that disappears is a field a client
/// reading `row.last_chunk_at_ms` sees as `undefined`.
#[test]
fn source_status_serde_shape_is_unchanged() {
    let json = serde_json::to_value(SourceStatus::idle("src_a".into())).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "source_id": "src_a",
            "chunks_synced": 0,
            "chunks_pending": 0,
            "last_chunk_at_ms": null,
            "freshness": "idle",
        })
    );
}
