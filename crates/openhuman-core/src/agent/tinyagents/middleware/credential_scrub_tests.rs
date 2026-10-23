//! The host's part of tool-result credential scrubbing: the browser
//! confirmation-token exemption, the fail-closed withhold, redaction counting,
//! and the chunking that keeps large results inside the module's request bound.
//! What the module actually removes is pinned against the released native
//! module in `modules/security_text_tests.rs`.

use super::*;

/// A stand-in for the module that replaces `secret-*` words, so the host logic
/// is tested without a module.
fn fake_module(text: &str) -> Option<String> {
    Some(
        text.split(' ')
            .map(|w| {
                if w.starts_with("secret-") {
                    "[REDACTED]"
                } else {
                    w
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

#[test]
fn unchanged_result_is_left_alone() {
    assert!(scrub_for_tool(&fake_module, "read_file", "plain text").is_none());
}

#[test]
fn changed_result_carries_the_model_facing_notice_and_count() {
    let (text, count) = scrub_for_tool(&fake_module, "read_file", "a secret-1 b secret-2").unwrap();
    assert_eq!(count, 2);
    assert!(text.starts_with("a [REDACTED] b [REDACTED]\n\n[credential_scrub] 2 value(s)"));
}

#[test]
fn placeholders_already_in_the_result_are_not_counted() {
    let (_, count) = scrub_for_tool(&fake_module, "read_file", "[REDACTED] secret-1").unwrap();
    assert_eq!(count, 1);
}

#[test]
fn a_module_that_cannot_answer_withholds_the_result_instead_of_passing_it_on() {
    let (text, count) = scrub_for_tool(&|_| None, "read_file", "password=hunter2secret").unwrap();
    assert_eq!(text, WITHHELD_NOTICE);
    assert_eq!(count, 1);
    assert!(!text.contains("hunter2"));
}

#[test]
fn browser_confirmation_token_survives_without_exempting_page_credentials() {
    let pending = serde_json::json!({
        "status": "NeedsConfirmation",
        "pending": {
            "action": {"action": "click", "target": {"kind": "ref", "value": "e33"}},
            "token": "00000000-0000-4000-8000-000000000123"
        },
        "page": {"text": "secret-pagecredential"}
    });
    // The fake module removes a whole word, so the page value must be a word.
    let redact = |text: &str| Some(text.replace("secret-pagecredential", "[REDACTED]"));
    let (scrubbed, count) = scrub_for_tool(&redact, "browser", &pending.to_string())
        .expect("page credential is still redacted");
    assert_eq!(count, 1);
    assert!(!scrubbed.contains("secret-pagecredential"));
    let body: serde_json::Value =
        serde_json::from_str(scrubbed.split("\n\n").next().unwrap()).unwrap();
    assert_eq!(body["pending"]["token"], pending["pending"]["token"]);
}

#[test]
fn browser_token_is_never_shown_to_the_module() {
    let seen = std::sync::Mutex::new(Vec::<String>::new());
    let redact = |text: &str| {
        seen.lock().unwrap().push(text.to_string());
        Some(text.to_string())
    };
    let token_only = serde_json::json!({
        "status": "NeedsConfirmation",
        "pending": {"token": "00000000-0000-4000-8000-000000000123"}
    });
    assert!(scrub_for_tool(&redact, "browser", &token_only.to_string()).is_none());
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .all(|text| !text.contains("00000000-0000-4000-8000-000000000123")));
}

#[test]
fn browser_result_is_withheld_when_the_module_cannot_answer() {
    let pending = serde_json::json!({
        "status": "NeedsConfirmation",
        "pending": {"token": "00000000-0000-4000-8000-000000000123"}
    });
    let (text, _) = scrub_for_tool(&|_| None, "browser", &pending.to_string()).unwrap();
    assert_eq!(text, WITHHELD_NOTICE);
}
