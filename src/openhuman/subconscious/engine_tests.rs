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

// ── Rate-cap circuit breaker (TAURI-RUST-HXF) ───────────────────────────

#[test]
fn evaluate_rate_cap_halt_skip_resume_proceed() {
    // No halt in effect → run normally.
    assert_eq!(
        evaluate_rate_cap_halt(None, "other:groq"),
        RateCapHaltDecision::Proceed
    );
    // Halt set for the same signature still in config → skip the doomed run.
    assert_eq!(
        evaluate_rate_cap_halt(Some("other:groq"), "other:groq"),
        RateCapHaltDecision::Skip
    );
    // Halt set but the user switched provider/model → clear it and resume.
    assert_eq!(
        evaluate_rate_cap_halt(Some("other:groq"), "cloud"),
        RateCapHaltDecision::Resume
    );
}

#[test]
fn permanent_rate_cap_error_matches_wrapped_groq_agent_error_only() {
    // The verbatim wrapped agent-run error the tick surfaces (413/TPM) →
    // permanent, so the breaker halts.
    assert!(is_permanent_rate_cap_error(
        r#"agent run: groq API error (413 Payload Too Large): {"error":{"message":"Request too large for model `openai/gpt-oss-120b` in organization `org_x` service tier `on_demand` on tokens per minute (TPM): Limit 8000, Requested 42084."}}"#
    ));
    // A transient 429 burst ("try again in Ns") must NOT halt — it stays
    // retryable, so the two permanent-error arms never overlap.
    assert!(!is_permanent_rate_cap_error(
        "agent run: groq API error (429 Too Many Requests): Rate limit reached. Please try again in 2.5s."
    ));
    // A tool-capability error is a different permanent condition handled by its
    // own arm, not the rate-cap breaker.
    assert!(!is_permanent_rate_cap_error(
        "agent run: No endpoints found that support tool use"
    ));
}

#[test]
fn subconscious_provider_signature_tracks_config_changes() {
    // Default config routes to OpenHuman cloud.
    let mut cfg = Config::default();
    assert_eq!(subconscious_provider_signature(&cfg), "cloud");

    // A BYO provider override yields a distinct, stable signature.
    cfg.subconscious_provider = Some("groq".to_string());
    let groq_sig = subconscious_provider_signature(&cfg);
    assert_eq!(groq_sig, "other:groq");

    // Switching the provider changes the signature — the breaker's cue to
    // clear a halt and resume ticking.
    cfg.subconscious_provider = Some("openai".to_string());
    assert_ne!(subconscious_provider_signature(&cfg), groq_sig);
}

#[test]
fn rate_cap_halt_state_transitions() {
    let mut state = EngineState {
        last_tick_at: 0.0,
        total_ticks: 0,
        consecutive_failures: 0,
        provider_unavailable_reason: None,
        rate_cap_halt_signature: None,
    };

    // No halt armed → the tick proceeds (does not skip).
    assert!(!state.should_skip_for_rate_cap_halt("other:groq"));

    // A permanent rate-cap failure arms the halt + actionable reason.
    state.arm_rate_cap_halt("other:groq");
    assert_eq!(state.rate_cap_halt_signature.as_deref(), Some("other:groq"));
    assert_eq!(
        state.provider_unavailable_reason.as_deref(),
        Some(RATE_CAP_HALT_REASON)
    );

    // Same config still set → skip the doomed run, and count the skipped tick.
    let before = state.total_ticks;
    assert!(state.should_skip_for_rate_cap_halt("other:groq"));
    assert_eq!(state.total_ticks, before + 1);

    // User switched provider (signature changed) → clear halt + reason, resume.
    assert!(!state.should_skip_for_rate_cap_halt("cloud"));
    assert!(state.rate_cap_halt_signature.is_none());
    assert!(state.provider_unavailable_reason.is_none());
}
