use super::*;
use crate::openhuman::agent::learning::candidate::{CueFamily, EvidenceRef, FacetClass};

fn extract(body: &str) -> Vec<LearningCandidate> {
    parse_signature(body, "gmail:test", "<msg-1@test.com>")
}

fn find<'a>(cs: &'a [LearningCandidate], key: &str) -> Option<&'a LearningCandidate> {
    cs.iter().find(|c| c.key == key)
}

#[test]
fn parse_signature_extracts_name_role_timezone_employer() {
    let body = "Hi, great to hear from you!\n\n\
                Please find the docs attached.\n\n\
                Thanks,\n\
                Alice Johnson\n\
                Senior Software Engineer\n\
                Acme Corp\n\
                San Francisco, CA\n\
                PST";
    let candidates = extract(body);

    let name = find(&candidates, "name").expect("name candidate");
    assert_eq!(name.value, "Alice Johnson");
    assert!((name.initial_confidence - CONF_NAME).abs() < 0.01);

    let role = find(&candidates, "role").expect("role candidate");
    assert!(role.value.contains("Engineer") || role.value.contains("engineer"));

    let tz = find(&candidates, "timezone").expect("timezone candidate");
    assert_eq!(tz.value, "PST");
    assert!((tz.initial_confidence - CONF_TIMEZONE).abs() < 0.01);

    let emp = find(&candidates, "employer").expect("employer candidate");
    assert!(emp.value.contains("Acme"));
}

#[test]
fn parse_signature_handles_no_signature() {
    let body = "just some content here nothing looks like a sig";
    let cs = extract(body);
    // No strong signals → no candidates emitted. The < 0.6 filter at the
    // tail of parse_signature drops anything below confidence threshold
    // (including the gated lone-location case), so the result must be empty.
    assert!(
        cs.is_empty(),
        "expected zero candidates from non-signature body, got {cs:?}"
    );
}

#[test]
fn parse_signature_ignores_quoted_replies() {
    // Even when the body has quoted sections, only the window of non-empty lines
    // at the very end is scanned.
    let body = "> On Monday, Alice wrote:\n\
                > Great to meet you!\n\
                >\n\
                Sure, let's connect.\n\n\
                Bob Smith\n\
                Product Manager\n\
                UTC+1";
    let cs = extract(body);
    let name = find(&cs, "name").expect("name candidate");
    assert_eq!(name.value, "Bob Smith");
    let tz = find(&cs, "timezone").expect("timezone");
    assert!(tz.value.starts_with("UTC"));
}

#[test]
fn parse_signature_low_confidence_for_lone_location() {
    // Location alone (no other strong signals) should NOT produce a candidate.
    let body = "The meeting is on Thursday.\nSan Francisco, CA";
    let cs = extract(body);
    assert!(
        find(&cs, "location").is_none(),
        "location candidate must not be emitted without other strong signals"
    );
}

#[test]
fn parse_signature_emits_evidence_email_message_variant() {
    let body = "Alice Smith\nCTO\nStartup Inc\nPST";
    let cs = extract(body);
    for c in &cs {
        assert!(
            matches!(
                &c.evidence,
                EvidenceRef::EmailMessage { source_id, message_id }
                if source_id == "gmail:test" && message_id == "<msg-1@test.com>"
            ),
            "expected EmailMessage evidence, got {:?}",
            c.evidence
        );
        assert_eq!(c.cue_family, CueFamily::Structural);
        assert_eq!(c.class, FacetClass::Identity);
    }
}
