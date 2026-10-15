//! The browser confirmation-token exemption: the one piece of credential
//! scrubbing policy the host owns. The scrubber, its notice and their tests
//! live in `tinyagents-harness` (`middleware::library::credential_scrub`).

use super::*;

#[test]
fn browser_confirmation_token_survives_without_exempting_page_credentials() {
    let pending = serde_json::json!({
        "status": "NeedsConfirmation",
        "pending": {
            "action": {"action": "click", "target": {"kind": "ref", "value": "e33"}},
            "token": "00000000-0000-4000-8000-000000000123"
        },
        "page": {
            "api_key": "sk-abcdefghijklmnopqrstuvwxyz123456",
            "text": "token=page-secret-value"
        }
    });
    let content = pending.to_string();
    let (scrubbed, count) =
        scrub_with_notice_for_tool("browser", &content).expect("page credential is still redacted");
    assert_eq!(count, 2);
    assert!(scrubbed.contains("00000000-0000-4000-8000-000000000123"));
    assert!(!scrubbed.contains("abcdefghijklmnopqrstuvwxyz123456"));
    assert!(!scrubbed.contains("page-secret-value"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(scrubbed.split("\n\n").next().unwrap()).unwrap()
            ["pending"]["token"],
        pending["pending"]["token"]
    );

    let token_only = serde_json::json!({
        "status": "NeedsConfirmation",
        "pending": {"token": "00000000-0000-4000-8000-000000000123"}
    });
    assert!(scrub_with_notice_for_tool("browser", &token_only.to_string()).is_none());
}

/// #6954: reading a source file that talks about tokens must reach the model
/// verbatim; only secret-looking values are redacted.
#[test]
fn source_code_about_tokens_passes_through_the_host_scrubber() {
    let source = r#"class Tokenizer:
    def __init__(self):
        self.eos_token = "<eos>"
        previous_token: Optional[int] = None

    def decode(self, token_id):
        token = self.vocab[int(token_id)]
        if token == self.unk_token:
            return score / max(token_count, 1)
        return {"mean_logprob_per_token": -4.73}
"#;
    assert!(
        scrub_with_notice_for_tool("read_file", source).is_none(),
        "source code was redacted"
    );

    let (scrubbed, count) = scrub_with_notice_for_tool("read_file", "password=hunter2secret")
        .expect("a real credential is still redacted");
    assert_eq!(count, 1);
    assert!(!scrubbed.contains("hunter2secret"));
}

#[test]
fn shared_security_corpus_pins_credential_middleware_catches_and_gaps() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/security-redaction-corpus.json"
    )))
    .unwrap();
    for case in corpus["text"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let output = scrub_with_notice_for_tool("read_file", input)
            .map(|(text, _)| text)
            .unwrap_or_else(|| input.to_owned());
        let removed = case["removed_by"]
            .as_array()
            .unwrap()
            .iter()
            .any(|redactor| redactor == "credential_middleware");
        assert_eq!(
            !output.contains(case["needle"].as_str().unwrap()),
            removed,
            "{}: {output}",
            case["case"]
        );
    }
}
