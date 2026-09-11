use super::*;
use std::collections::HashSet;

fn composio_entry(id: &str, connection_id: &str) -> MemorySourceEntry {
    MemorySourceEntry {
        id: id.to_string(),
        kind: SourceKind::Composio,
        label: format!("Gmail · {connection_id}"),
        enabled: true,
        toolkit: Some("gmail".to_string()),
        connection_id: Some(connection_id.to_string()),
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
        max_items: Some(100),
        selector: None,
        max_tokens_per_sync: None,
        max_cost_per_sync_usd: None,
        sync_depth_days: Some(30),
    }
}

fn local_entry(id: &str) -> MemorySourceEntry {
    MemorySourceEntry {
        id: id.to_string(),
        kind: SourceKind::Folder,
        label: "Notes".to_string(),
        enabled: true,
        toolkit: None,
        connection_id: None,
        path: Some("/tmp/notes".to_string()),
        glob: None,
        url: None,
        branch: None,
        paths: Vec::new(),
        max_commits: None,
        max_issues: None,
        max_prs: None,
        query: None,
        since_days: None,
        max_items: None,
        selector: None,
        max_tokens_per_sync: None,
        max_cost_per_sync_usd: None,
        sync_depth_days: None,
    }
}

fn active_set(ids: &[&str]) -> HashSet<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

/// RC-A: an inactive connection's row is hidden, only the active one shows —
/// but the input list is preserved (hide-not-delete; the filter never removes
/// entries from `config.memory_sources`, it only subtracts from the view).
#[test]
fn hides_inactive_connection_keeps_active() {
    let sources = vec![
        composio_entry("src_a", "conn_A"), // inactive
        composio_entry("src_b", "conn_B"), // active
    ];
    let active = active_set(&["conn_B"]);
    let out = filter_to_active_composio_sources(sources.clone(), Some(&active));

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].connection_id.as_deref(), Some("conn_B"));
    // Both rows still exist in the original list — nothing was deleted.
    assert_eq!(sources.len(), 2);
}

/// RC-B: two rows with the same active connection_id collapse to one.
#[test]
fn dedupes_identical_connection_ids() {
    let sources = vec![
        composio_entry("src_1", "conn_B"),
        composio_entry("src_2", "conn_B"),
    ];
    let active = active_set(&["conn_B"]);
    let out = filter_to_active_composio_sources(sources, Some(&active));

    assert_eq!(out.len(), 1, "identical-id rows must collapse to one");
    assert_eq!(out[0].connection_id.as_deref(), Some("conn_B"));
}

/// A previously-inactive connection reappears (with its settings intact) once
/// it is back in the active set — confirming hiding is transient, not removal.
#[test]
fn reactivated_connection_reappears_with_settings() {
    let sources = vec![composio_entry("src_a", "conn_A")];
    let active = active_set(&["conn_A"]);
    let out = filter_to_active_composio_sources(sources, Some(&active));

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].connection_id.as_deref(), Some("conn_A"));
    // Settings (caps) survive the round-trip — the row was never mutated.
    assert_eq!(out[0].max_items, Some(100));
    assert_eq!(out[0].sync_depth_days, Some(30));
}

/// Non-Composio sources have no connection and are always shown, regardless
/// of the active set.
#[test]
fn non_composio_sources_always_shown() {
    let sources = vec![local_entry("src_local"), composio_entry("src_x", "conn_X")];
    // Active set excludes the composio connection entirely.
    let active = active_set(&[]);
    let out = filter_to_active_composio_sources(sources, Some(&active));

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, SourceKind::Folder);
}

/// Safety: when the scan was unavailable (`None`), the list passes through
/// untouched — we must never hide every Composio source on a transient blip.
#[test]
fn none_active_set_shows_all() {
    let sources = vec![
        composio_entry("src_a", "conn_A"),
        composio_entry("src_b", "conn_B"),
        local_entry("src_local"),
    ];
    let out = filter_to_active_composio_sources(sources, None);
    assert_eq!(out.len(), 3, "failed scan must show all sources, hide none");
}

/// A Composio row missing its connection_id is kept rather than silently
/// dropped, even when an active set is present.
#[test]
fn composio_without_connection_id_is_kept() {
    let mut orphan = composio_entry("src_orphan", "conn_unused");
    orphan.connection_id = None;
    let active = active_set(&["conn_B"]);
    let out = filter_to_active_composio_sources(vec![orphan], Some(&active));
    assert_eq!(out.len(), 1);
}
