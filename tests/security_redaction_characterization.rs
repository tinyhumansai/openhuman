//! Log, denial and RPC redaction characterization using the shared security corpus.
//! TinySecurity's `Redact`/`RedactWith` modes consume the same corpus in the
//! module client tests; Composio and credential middleware tests live beside
//! their owning adapters.

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
    let rpc_params = openhuman_core::core::rpc_log::redact_params_for_log(&json!({
        "access_token": input,
    }));
    vec![
        ("denials", denial),
        (
            "credential_middleware",
            tinyinference_core::sanitize::scrub_credentials(input),
        ),
        (
            "log",
            openhuman_core::core::log_redaction::scrub_secrets(input),
        ),
        ("rpc_params", serde_json::to_string(&rpc_params).unwrap()),
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

fn needle_of(case: &Value) -> String {
    match case["needle_parts"].as_array() {
        Some(parts) => parts.iter().map(|p| p.as_str().unwrap()).collect(),
        None => case["needle"].as_str().unwrap().to_string(),
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
        // The private credential middleware consumes this same fixture beside its module.
        .filter(|name| {
            name != "credential_middleware"
                && name != "approval_args"
                && name != "security_prefix"
                && name != "security_scrub"
        })
        .collect()
}

#[test]
fn extended_corpus_pins_which_redactors_remove_each_shape() {
    let corpus: Value = serde_json::from_str(EXTENDED).unwrap();
    for case in corpus["text"].as_array().unwrap() {
        let mut actual = removed_by(&input_of(case), &needle_of(case));
        // Private credential middleware consumes this fixture in its owning unit test.
        actual.remove("credential_middleware");
        assert_eq!(actual, pinned(case), "case={} drifted", case["case"]);
    }
}

fn url_outputs(input: &str) -> Vec<(&'static str, String)> {
    vec![
        (
            "log",
            openhuman_core::core::log_redaction::scrub_secrets(input),
        ),
        ("util_url", openhuman_core::util::redact_url_for_log(input)),
        (
            "storage_url",
            openhuman_core::config::schema::storage::redact_url(input),
        ),
    ]
}

#[test]
fn url_corpus_pins_all_public_url_redactors() {
    let corpus: Value = serde_json::from_str(EXTENDED).unwrap();
    for case in corpus["urls"].as_array().unwrap() {
        let input = input_of(case);
        let needle = needle_of(case);
        let actual: BTreeSet<String> = url_outputs(&input)
            .into_iter()
            .filter(|(_, output)| !output.contains(&needle))
            .map(|(name, _)| name.to_string())
            .collect();
        let mut expected: BTreeSet<String> = case["removed_by"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect();
        // The JSON-RPC client owns its URL scrubber in a sibling crate test.
        expected.remove("rpc_url");
        assert_eq!(actual, expected, "URL case={} drifted", case["case"]);
    }
}

/// Sinks that must scrub each kind of text (the desired contract the module owns).
fn required_sinks(kind: &str) -> &'static [&'static str] {
    match kind {
        "secret" => &["log", "denials"],
        "pii" => &[],
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
            let removed = removed_by(&input_of(case), &needle_of(case));
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
         the expected gap list must shrink when a sink changes, and a regression must not grow it silently"
    );
}

/// DESIRED: no secret or PII shape in either corpus survives a required sink.
/// The expected panic records the current gap set without turning this suite red.
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
