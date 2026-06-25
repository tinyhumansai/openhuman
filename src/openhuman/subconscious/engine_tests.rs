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
