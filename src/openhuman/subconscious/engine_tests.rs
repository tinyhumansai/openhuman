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
