//! The walk of a Composio outcome around the module's `RedactWith`: every string
//! channel is sent for checking, answers are written back (object keys included,
//! colliding keys kept apart), and a result the module could not check is
//! withheld. What the module removes is pinned against the released archive in
//! `modules/security_text_tests.rs`.

use std::collections::HashMap;

use serde_json::json;

use crate::config::{ComposioHostCredential, Config};
use crate::integrations::composio::tools::redact::{
    composio_secrets, redact_and_report, redact_composio_outcome, redact_outcome_with, Answers,
    WITHHELD,
};

const SHORT_KEY: &str = "abcdefgh";
const LONG_KEY: &str = "abcdefghi";

fn keys() -> Vec<String> {
    vec![LONG_KEY.to_string(), SHORT_KEY.to_string()]
}

fn config_with_overlapping_secrets() -> Config {
    let tmp = tempfile::tempdir().expect("tempdir for config_with_overlapping_secrets");
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.api_key = Some(LONG_KEY.to_string());
    config
        .composio
        .pin_host_credential(ComposioHostCredential::direct(SHORT_KEY));
    std::mem::forget(tmp);
    config
}

/// A stand-in for the module: replaces the exact secrets, longest first, like
/// the module's registry does.
async fn replace_longest_first(strings: Vec<String>) -> Answers {
    let secrets = [LONG_KEY, SHORT_KEY];
    Ok(strings
        .into_iter()
        .map(|text| {
            let redacted = secrets
                .iter()
                .fold(text.clone(), |acc, key| acc.replace(key, "[REDACTED]"));
            (text, redacted)
        })
        .collect())
}

async fn cannot_answer(_strings: Vec<String>) -> Answers {
    Err("TinySecurity RedactWith failed".to_string())
}

fn texts(result: &tinytools::ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match c {
            tinytools::ToolContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn the_host_resolves_every_key_a_call_can_use_longest_first() {
    let secrets = composio_secrets(&config_with_overlapping_secrets());
    assert_eq!(secrets, vec![LONG_KEY.to_string(), SHORT_KEY.to_string()]);
}

#[tokio::test]
async fn overlapping_secrets_redact_longest_first_in_text() {
    let outcome = redact_outcome_with(
        Ok(tinytools::ToolResult::success(format!(
            "token {LONG_KEY} in transit"
        ))),
        &keys(),
        replace_longest_first,
    )
    .await
    .unwrap();
    assert_eq!(texts(&outcome), "token [REDACTED] in transit");
}

/// A secret that happened to be used as a JSON object key must not survive.
#[tokio::test]
async fn secret_used_as_a_json_object_key_is_redacted() {
    let outcome = redact_outcome_with(
        Ok(tinytools::ToolResult::json(
            json!({ format!("token_{LONG_KEY}"): "value" }),
        )),
        &keys(),
        replace_longest_first,
    )
    .await
    .unwrap();
    let serialized = serde_json::to_string(&outcome).unwrap();
    assert!(!serialized.contains(LONG_KEY), "{serialized}");
    assert!(serialized.contains("[REDACTED]"), "{serialized}");
}

#[tokio::test]
async fn colliding_redacted_object_keys_keep_both_values() {
    let outcome = redact_outcome_with(
        Ok(tinytools::ToolResult::json(json!({
            (SHORT_KEY): "short value",
            (LONG_KEY): "long value",
        }))),
        &keys(),
        replace_longest_first,
    )
    .await
    .unwrap();
    let tinytools::ToolContent::Json { data } = &outcome.content[0] else {
        panic!("expected JSON content");
    };
    let object = data.as_object().expect("JSON object");
    assert_eq!(object.len(), 2);
    let mut values: Vec<_> = object.values().map(|v| v.as_str().unwrap()).collect();
    values.sort_unstable();
    assert_eq!(values, vec!["long value", "short value"]);
    assert!(object.keys().all(|key| key.starts_with("[REDACTED]")));
}

/// Every distinct result string is sent for checking, including keys and values
/// that do not contain this call's Composio credential.
#[tokio::test]
async fn every_result_string_is_sent_to_the_module() {
    use std::sync::Mutex;
    let seen = Mutex::new(Vec::<Vec<String>>::new());
    let redactor = |strings: Vec<String>| {
        seen.lock().unwrap().push(strings.clone());
        async move {
            Ok(strings
                .into_iter()
                .map(|s| (s.clone(), s.replace(LONG_KEY, "[REDACTED]")))
                .collect::<HashMap<_, _>>())
        }
    };
    let result = redact_outcome_with(
        Ok(tinytools::ToolResult::json(
            json!({"clean": "a long enough value", "dirty": format!("has {LONG_KEY} inside")}),
        )),
        &keys(),
        &redactor,
    )
    .await
    .unwrap();
    let serialized = serde_json::to_string(&result).unwrap();
    assert!(serialized.contains("a long enough value"));
    assert!(!serialized.contains(LONG_KEY), "{serialized}");
    let sent = seen.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0],
        vec![
            "a long enough value".to_string(),
            "clean".to_string(),
            "dirty".to_string(),
            format!("has {LONG_KEY} inside"),
        ]
    );

    // Even a result with no configured key is withheld if the module cannot
    // check it for provider credentials.
    let withheld = redact_outcome_with(
        Ok(tinytools::ToolResult::success("nothing sensitive here")),
        &keys(),
        cannot_answer,
    )
    .await
    .unwrap_err();
    assert_eq!(withheld.to_string(), WITHHELD);
}

#[tokio::test]
async fn an_oversized_result_is_withheld_before_module_calls() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    let calls = Arc::new(AtomicUsize::new(0));
    let called = Arc::clone(&calls);
    let values = (0..257).map(|index| format!("result field {index}"));
    let outcome = redact_outcome_with(
        Ok(tinytools::ToolResult::json(json!(
            values.collect::<Vec<_>>()
        ))),
        &keys(),
        move |strings| {
            called.fetch_add(1, Ordering::SeqCst);
            async move {
                Ok(strings
                    .into_iter()
                    .map(|text| (text.clone(), text))
                    .collect())
            }
        },
    )
    .await;

    assert_eq!(outcome.unwrap_err().to_string(), WITHHELD);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_result_without_its_composio_key_still_redacts_other_credentials() {
    const PROVIDER_TOKEN: &str = "bearer provider-token-abcdef";
    let outcome = redact_outcome_with(
        Ok(tinytools::ToolResult::success(format!(
            "provider response: {PROVIDER_TOKEN}"
        ))),
        &keys(),
        |strings| async move {
            Ok(strings
                .into_iter()
                .map(|text| {
                    let redacted = text.replace(PROVIDER_TOKEN, "bearer [REDACTED]");
                    (text, redacted)
                })
                .collect())
        },
    )
    .await
    .unwrap();

    assert_eq!(texts(&outcome), "provider response: bearer [REDACTED]");
}

#[tokio::test]
async fn a_result_the_module_could_not_check_is_withheld() {
    let error = redact_outcome_with(
        Ok(tinytools::ToolResult::success(format!(
            "token {LONG_KEY} in transit"
        ))),
        &keys(),
        cannot_answer,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), WITHHELD);
    assert!(!error.to_string().contains(LONG_KEY));
}

#[tokio::test]
async fn a_module_answer_that_keeps_a_configured_key_is_withheld() {
    let error = redact_outcome_with(
        Ok(tinytools::ToolResult::success(format!("key {LONG_KEY}"))),
        &keys(),
        |strings| async move {
            Ok(strings
                .into_iter()
                .map(|text| (text.clone(), text))
                .collect())
        },
    )
    .await
    .unwrap_err();

    assert_eq!(error.to_string(), WITHHELD);
    assert!(!error.to_string().contains(LONG_KEY));
}

#[tokio::test]
async fn an_error_text_is_redacted_or_withheld_never_passed_on() {
    let redacted = redact_outcome_with(
        Err(anyhow::anyhow!("bad credential {LONG_KEY}")),
        &keys(),
        replace_longest_first,
    )
    .await
    .unwrap_err();
    assert_eq!(redacted.to_string(), "bad credential [REDACTED]");
    let withheld = redact_outcome_with(
        Err(anyhow::anyhow!("bad credential {LONG_KEY}")),
        &keys(),
        cannot_answer,
    )
    .await
    .unwrap_err();
    assert_eq!(withheld.to_string(), WITHHELD);
    // Key-free error text is still checked and withheld when the module cannot
    // answer.
    let short = redact_outcome_with(Err(anyhow::anyhow!("nope")), &keys(), cannot_answer)
        .await
        .unwrap_err();
    assert_eq!(short.to_string(), WITHHELD);
}

/// With the module switched off, the production entry points withhold all result
/// and error text, including strings without a configured Composio key.
#[tokio::test]
async fn with_the_module_disabled_the_production_entry_points_withhold() {
    let mut config = config_with_overlapping_secrets();
    config.modules.enabled = false;
    let outcome = redact_composio_outcome(
        &config,
        Ok(tinytools::ToolResult::success(format!("key {LONG_KEY}"))),
    )
    .await
    .unwrap_err();
    assert_eq!(outcome.to_string(), WITHHELD);
    let rendered = format!("[composio-direct] failed: invalid credential {LONG_KEY}");
    let reported = redact_and_report(&config, "test_op", &rendered).await;
    assert_eq!(reported, WITHHELD);
    assert!(!reported.contains(LONG_KEY) && !reported.contains(SHORT_KEY));
    let unrelated = redact_composio_outcome(
        &config,
        Ok(tinytools::ToolResult::success(
            "provider response: bearer provider-token-abcdef",
        )),
    )
    .await
    .unwrap_err();
    assert_eq!(unrelated.to_string(), WITHHELD);
    let uncredentialed_error = redact_and_report(
        &config,
        "test_op",
        "provider error: bearer provider-token-abcdef",
    )
    .await;
    assert_eq!(uncredentialed_error, WITHHELD);
}

/// Without a configured key, an unavailable module still means the host cannot
/// prove that a result contains no other credentials.
#[tokio::test]
async fn an_unconfigured_call_is_withheld_when_the_module_is_unavailable() {
    let mut config = Config::default();
    let tmp = tempfile::tempdir().unwrap();
    config.config_path = tmp.path().join("config.toml");
    config.modules.enabled = false;
    let outcome = redact_composio_outcome(
        &config,
        Ok(tinytools::ToolResult::success(
            "provider response: bearer provider-token-abcdef",
        )),
    )
    .await
    .unwrap_err();
    assert_eq!(outcome.to_string(), WITHHELD);
}
