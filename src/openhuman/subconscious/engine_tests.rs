use super::*;
use crate::openhuman::subconscious::reflection::ReflectionKind;

#[test]
fn parse_thoughts_from_envelope() {
    let json = r#"{"thoughts": [
        {"kind": "hotness_spike", "body": "Phoenix surged", "source_refs": ["entity:phoenix"]},
        {"kind": "risk", "body": "Deadline approaching"}
    ]}"#;
    let drafts = parse_thoughts(json);
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts[0].kind, ReflectionKind::HotnessSpike);
    assert_eq!(drafts[1].kind, ReflectionKind::Risk);
}

#[test]
fn parse_thoughts_from_reflections_key() {
    let json = r#"{"reflections": [
        {"kind": "opportunity", "body": "New connection available"}
    ]}"#;
    let drafts = parse_thoughts(json);
    assert_eq!(drafts.len(), 1);
}

#[test]
fn parse_thoughts_from_bare_array() {
    let json = r#"[{"kind": "daily_digest", "body": "Summary of the day"}]"#;
    let drafts = parse_thoughts(json);
    assert_eq!(drafts.len(), 1);
}

#[test]
fn parse_thoughts_returns_empty_on_garbage() {
    let drafts = parse_thoughts("not json at all");
    assert!(drafts.is_empty());
}

#[test]
fn parse_thoughts_handles_markdown_wrapper() {
    let json = "```json\n{\"thoughts\": [{\"kind\": \"risk\", \"body\": \"test\"}]}\n```";
    let drafts = parse_thoughts(json);
    assert_eq!(drafts.len(), 1);
}

#[test]
fn extract_json_finds_object() {
    let text = "Here's the JSON: {\"a\": 1} done.";
    let extracted = extract_json(text);
    assert!(extracted.starts_with('{'));
    assert!(extracted.ends_with('}'));
}

#[test]
fn extract_json_finds_array() {
    let text = "Result: [1, 2, 3] end.";
    let extracted = extract_json(text);
    assert!(extracted.starts_with('['));
    assert!(extracted.ends_with(']'));
}

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
