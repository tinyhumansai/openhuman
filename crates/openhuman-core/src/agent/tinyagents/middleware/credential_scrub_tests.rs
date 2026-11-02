//! The module's shared Redact modes are verified against the released fixture.

use super::*;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads and loads the pinned native TinySecurity release"]
async fn native_credentials_only_and_browser_result_adapter() {
    let mut config = crate::config::Config::default();
    config.modules.enabled = true;
    crate::modules::security::ensure_loaded(&config)
        .await
        .expect("the pinned TinySecurity module should load");

    let pending = serde_json::json!({
        "status": "NeedsConfirmation",
        "pending": {"token": "00000000-0000-4000-8000-000000000123"},
        "page": {
            "api_key": "sk-abcdefghijklmnopqrstuvwxyz123456",
            "text": "token=page-secret-value"
        }
    });
    let content = pending.to_string();
    let (scrubbed, _) = scrub_with_notice_for_tool("browser", &content)
        .expect("page credentials should be redacted");
    assert!(
        scrubbed.contains("00000000-0000-4000-8000-000000000123"),
        "{scrubbed}"
    );
    assert!(!scrubbed.contains("abcdefghijklmnopqrstuvwxyz123456"));
    assert!(!scrubbed.contains("page-secret-value"));

    let source = "class Tokenizer: eos_token = '<eos>'";
    assert!(scrub_with_notice_for_tool("read_file", source).is_none());
    let (scrubbed, _) = scrub_with_notice_for_tool("read_file", "password=hunter2secret")
        .expect("credential should be redacted");
    assert!(!scrubbed.contains("hunter2secret"));

    let corpus: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/security-redaction-corpus.json"
    )))
    .unwrap();
    for case in corpus["text"].as_array().unwrap() {
        let expected = case["removed_by"]
            .as_array()
            .unwrap()
            .iter()
            .any(|sink| sink.as_str() == Some("credential_middleware"));
        if !expected {
            continue;
        }
        let input = fixture_parts(case, "input_parts", "input");
        let needle = fixture_parts(case, "needle_parts", "needle");
        let actual = scrub_with_notice_for_tool("read_file", &input);
        assert!(actual.is_some(), "case={} did not redact", case["case"]);
        assert!(
            !actual.unwrap().0.contains(&needle),
            "case={} retained its needle",
            case["case"]
        );
    }
}

fn fixture_parts(case: &serde_json::Value, parts: &str, fallback: &str) -> String {
    case[parts]
        .as_array()
        .map(|values| values.iter().map(|value| value.as_str().unwrap()).collect())
        .unwrap_or_else(|| case[fallback].as_str().unwrap().to_owned())
}
