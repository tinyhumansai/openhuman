use super::*;

// ── Tick origin upgrade (#approval-origin) ──────────────────────────────

#[test]
fn tick_origin_untainted_keeps_subconscious_source() {
    use crate::openhuman::agent::turn_origin::TrustedAutomationSource;
    let source = tick_origin_source(false);
    assert!(matches!(source, TrustedAutomationSource::Subconscious));
}

#[test]
fn tick_origin_with_external_sync_chunk_uses_tainted_source() {
    use crate::openhuman::agent::turn_origin::TrustedAutomationSource;
    let source = tick_origin_source(true);
    assert!(matches!(
        source,
        TrustedAutomationSource::SubconsciousTainted
    ));
}

// ── Tool-capability error detection (TAURI-RUST-ADC) ────────────────────

#[test]
fn tool_capability_error_matches_openrouter_and_direct_bodies() {
    // OpenRouter router-level 404 (the reported ADC body).
    assert!(is_tool_capability_error(
        r#"agent run: openrouter API error (404 Not Found): {"error":{"message":"No endpoints found that support tool use. Try disabling \"spawn_async_subagent\"."}}"#
    ));
    // Direct-provider "does not support tools" phrasing (TAURI-RUST-35 family).
    assert!(is_tool_capability_error(
        r#"agent run: cloud API error: {"error":{"message":"qwen2:0.5b does not support tools"}}"#
    ));
    // Case-insensitive.
    assert!(is_tool_capability_error(
        "NO ENDPOINTS FOUND THAT SUPPORT TOOL USE"
    ));
}

#[test]
fn tool_capability_error_ignores_unrelated_failures() {
    // A different 404, an auth wall, and a generic timeout must NOT match.
    assert!(!is_tool_capability_error(
        r#"agent run: openrouter API error (404 Not Found): {"error":{"message":"model 'llama3.3' not found"}}"#
    ));
    assert!(!is_tool_capability_error(
        "agent run: Backend returned 401 Unauthorized: Invalid token"
    ));
    assert!(!is_tool_capability_error("agent run: request timed out"));
}

// ── World-diff rendering (Stage 1) ──────────────────────────────────────

use crate::openhuman::memory_diff::types::{
    ChangeKind, CrossSourceDiff, DiffResult, DiffSummary, ItemChange,
};

fn change(item_id: &str, title: &str, kind: ChangeKind) -> ItemChange {
    ItemChange {
        item_id: item_id.to_string(),
        title: title.to_string(),
        kind,
        old_content_hash: None,
        new_content_hash: None,
        text_diff: None,
    }
}

#[test]
fn empty_cross_source_diff_has_zero_change_count() {
    let diff = CrossSourceDiff {
        checkpoint_id: Some("ckpt_1".into()),
        computed_at_ms: 0,
        summary: DiffSummary::default(),
        per_source: Vec::new(),
    };
    assert_eq!(world_diff_change_count(&diff), 0);
    // The "no changes" render is the quiet-tick sentinel; the tick short-circuits
    // before it ever reaches the agent, but the renderer stays well-defined.
    assert!(render_world_diff(&diff).contains("Nothing changed"));
}

#[test]
fn render_world_diff_summarises_changes_per_source() {
    let diff = CrossSourceDiff {
        checkpoint_id: Some("ckpt_1".into()),
        computed_at_ms: 0,
        summary: DiffSummary {
            added: 2,
            modified: 1,
            removed: 0,
            unchanged: 5,
        },
        per_source: vec![DiffResult {
            source_id: "src_gmail".into(),
            source_kind: "composio".into(),
            source_label: "Gmail".into(),
            from_snapshot_id: Some("snap_a".into()),
            to_snapshot_id: "snap_b".into(),
            summary: DiffSummary {
                added: 2,
                modified: 1,
                removed: 0,
                unchanged: 5,
            },
            changes: vec![
                change("m1", "Invoice from Acme", ChangeKind::Added),
                change("m2", "Re: launch plan", ChangeKind::Added),
                change("m3", "Standup notes", ChangeKind::Modified),
            ],
        }],
    };

    assert_eq!(world_diff_change_count(&diff), 3);
    let rendered = render_world_diff(&diff);
    assert!(rendered.contains("3 item(s) changed"));
    assert!(rendered.contains("Gmail (composio)"));
    assert!(rendered.contains("[added] Invoice from Acme"));
    assert!(rendered.contains("[modified] Standup notes"));
}

#[test]
fn render_world_diff_caps_items_and_falls_back_to_item_id() {
    let mut changes = Vec::new();
    for i in 0..(MAX_ITEMS_PER_SOURCE + 3) {
        // Empty title forces the item_id fallback.
        changes.push(change(&format!("item_{i}"), "", ChangeKind::Added));
    }
    let n = changes.len() as u32;
    let diff = CrossSourceDiff {
        checkpoint_id: None,
        computed_at_ms: 0,
        summary: DiffSummary {
            added: n,
            ..DiffSummary::default()
        },
        per_source: vec![DiffResult {
            source_id: "src_folder".into(),
            source_kind: "folder".into(),
            source_label: "Notes".into(),
            from_snapshot_id: None,
            to_snapshot_id: "snap_x".into(),
            summary: DiffSummary {
                added: n,
                ..DiffSummary::default()
            },
            changes,
        }],
    };

    let rendered = render_world_diff(&diff);
    assert!(rendered.contains("[added] item_0"), "uses item_id fallback");
    assert!(rendered.contains("…and 3 more"), "caps the per-source list");
}
