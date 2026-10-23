//! Phase-0 redaction characterization (issue #7328, plan Task 1).
//!
//! Two corpora feed every host redactor reachable from a root test:
//! `security-redaction-corpus.json` (also consumed beside the private Composio and
//! credential-scrub middleware, and the four URL helpers in `config`, `inference`
//! and `web3`) and `security-redaction-extended-corpus.json`, which adds more
//! secret and PII shapes. Redactors covered here:
//! `tinymemory_api::safety` (pii), `core/log_redaction.rs`
//! (the floor every URL log field now uses) and `tools/registry/denials.rs`. The
//! prefix, identity-hash, approval-argument, `security/scrub.rs` and URL helpers were deleted in
//! favour of the module's `security::redact`, which `modules::security` tests
//! pin against the released archive. Each case pins which redactors remove its needle *today*
//! (`removed_by`). A secret that a sink still leaks is NOT blessed: the leak is
//! recorded as a known gap, and `desired_*` tests state the wanted behaviour and
//! stay red (`should_panic(DESIRED-RED)`) until the module-owned redactor lands.

use std::collections::BTreeSet;

use serde_json::{json, Value};

const BASE: &str = include_str!("fixtures/security-redaction-corpus.json");
const EXTENDED: &str = include_str!("fixtures/security-redaction-extended-corpus.json");

/// Every redactor, as (name, function). Names match `removed_by` entries.
fn outputs(input: &str) -> Vec<(&'static str, String)> {
    // The denial log is process-global and read back as "the latest entry".
    static DENIAL_LOG: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _serial = DENIAL_LOG.lock().unwrap_or_else(|e| e.into_inner());
    tinytools::registry::denials::record("characterization", "test", "blocked", input);
    let denial = tinytools::registry::denials::list(1).pop().unwrap().reason;
    vec![
        ("denials", denial),
        (
            "log",
            openhuman_core::core::log_redaction::scrub_secrets(input),
        ),
        ("pii", tinymemory_api::safety::redact_identifiers(input)),
    ]
}

/// A case's input; secret-shaped literals are stored split (`input_parts`) so no
/// credential-shaped string is committed in the fixture.
fn input_of(case: &Value) -> String {
    match case["input_parts"].as_array() {
        Some(parts) => parts.iter().map(|p| p.as_str().unwrap()).collect(),
        None => case["input"].as_str().unwrap().to_string(),
    }
}

fn removed_by(input: &str, needle: &str) -> BTreeSet<String> {
    outputs(input)
        .into_iter()
        .filter(|(_, out)| !out.contains(needle))
        .map(|(name, _)| name.to_string())
        .collect()
}

fn pinned(case: &Value) -> BTreeSet<String> {
    case["removed_by"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        // `composio_*` / `credential_middleware` are asserted beside their private modules.
        .filter(|name| name != "credential_middleware")
        .collect()
}

#[test]
fn extended_corpus_pins_which_redactors_remove_each_shape() {
    let corpus: Value = serde_json::from_str(EXTENDED).unwrap();
    for case in corpus["text"].as_array().unwrap() {
        let actual = removed_by(&input_of(case), case["needle"].as_str().unwrap());
        assert_eq!(
            actual,
            pinned(case),
            "case={} drifted; removed_by is now {actual:?}",
            case["case"]
        );
    }
}

#[test]
fn extended_url_corpus_is_cleaned_by_the_log_floor() {
    let corpus: Value = serde_json::from_str(EXTENDED).unwrap();
    for case in corpus["urls"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let out = openhuman_core::core::log_redaction::scrub_secrets(input);
        assert!(
            !out.contains(case["needle"].as_str().unwrap()),
            "floor kept the secret in {} -> {out}",
            case["case"]
        );
    }
}

/// Sinks that must scrub each kind of text (the desired contract the module owns).
fn required_sinks(kind: &str) -> &'static [&'static str] {
    match kind {
        "secret" => &["log", "denials"],
        "pii" => &["pii"],
        _ => &[],
    }
}

/// `(case, sink)` pairs where a secret or PII needle survives a required sink.
fn known_gaps() -> BTreeSet<(String, String)> {
    let mut gaps = BTreeSet::new();
    for source in [BASE, EXTENDED] {
        let corpus: Value = serde_json::from_str(source).unwrap();
        for case in corpus["text"].as_array().unwrap() {
            // The base corpus has no `kind`; infer it from the case name.
            let name = case["case"].as_str().unwrap();
            let kind = case["kind"].as_str().unwrap_or(match name {
                "anthropic_key" => "secret",
                "email" => "pii",
                _ => "path",
            });
            let removed = removed_by(&input_of(case), case["needle"].as_str().unwrap());
            for sink in required_sinks(kind) {
                if !removed.contains(*sink) {
                    gaps.insert((name.to_string(), sink.to_string()));
                }
            }
        }
    }
    gaps
}

#[test]
fn known_missed_secret_gaps_are_recorded_exactly() {
    let expected: BTreeSet<(String, String)> = serde_json::from_str::<Vec<(String, String)>>(
        include_str!("fixtures/security-redaction-known-gaps.json"),
    )
    .unwrap()
    .into_iter()
    .collect();
    assert_eq!(
        known_gaps(),
        expected,
        "the set of redactor gaps changed: a fix must shrink the committed list (and \
         eventually delete the desired_* should_panic), a regression must not grow it silently"
    );
}

/// DESIRED: no secret or PII shape in either corpus survives a required sink.
/// Red until the single module-owned redactor replaces the nine host copies.
#[test]
#[should_panic(expected = "DESIRED-RED")]
fn desired_every_required_sink_removes_every_secret_and_pii_shape() {
    let gaps = known_gaps();
    assert!(
        gaps.is_empty(),
        "DESIRED-RED: {} missed-secret cases: {gaps:?}",
        gaps.len()
    );
}

#[test]
fn ordinary_text_is_never_redacted_by_the_log_scrubber() {
    // False-positive guard: harmless prose passes through the log and denial sinks.
    let outs = outputs("hello world");
    for (name, out) in outs {
        if matches!(name, "log" | "denials") {
            assert!(
                out.contains("hello world"),
                "{name} mangled ordinary text: {out}"
            );
        }
    }
}
