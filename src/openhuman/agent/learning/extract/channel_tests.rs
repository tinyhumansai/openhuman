use super::*;
use crate::openhuman::agent::learning::candidate::{self, FacetClass};

#[test]
fn normalize_rejects_empty_and_internal() {
    assert_eq!(normalize_channel_id(""), None);
    assert_eq!(normalize_channel_id("   "), None);
    assert_eq!(normalize_channel_id("internal"), None);
    assert_eq!(normalize_channel_id("INTERNAL"), None);
}

#[test]
fn normalize_lowercases() {
    assert_eq!(
        normalize_channel_id("Web_Chat").as_deref(),
        Some("web_chat")
    );
    assert_eq!(
        normalize_channel_id("desktop-chat").as_deref(),
        Some("desktop-chat")
    );
}

#[test]
fn emit_primary_channel_pushes_structural_candidate() {
    let _ = candidate::global().drain();
    assert!(emit_primary_channel("desktop-chat"));
    let drained = candidate::global().drain();
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].class, FacetClass::Channel);
    assert_eq!(drained[0].key, "primary");
    assert_eq!(drained[0].value, "desktop-chat");
    assert_eq!(drained[0].cue_family, CueFamily::Structural);
}

#[test]
fn emit_primary_channel_skips_internal() {
    let _ = candidate::global().drain();
    assert!(!emit_primary_channel("internal"));
    assert!(candidate::global().drain().is_empty());
}
