use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse};

use super::*;
use crate::inference::provider::factory::test_provider_override;
use crate::web_chat::subscribe_web_channel_events;

// ── parse_suggestions / strip_markdown_fence: pure unit tests ──────────────

#[test]
fn parses_a_bare_json_array() {
    let raw = r#"[{"prompt": "What about X?", "label": "About X"}, {"prompt": "And Y?"}]"#;
    let suggestions = parse_suggestions(raw).expect("valid json");
    assert_eq!(suggestions.len(), 2);
    assert_eq!(suggestions[0].prompt, "What about X?");
    assert_eq!(suggestions[0].label.as_deref(), Some("About X"));
    assert_eq!(suggestions[1].prompt, "And Y?");
    assert_eq!(suggestions[1].label, None);
}

#[test]
fn strips_a_json_markdown_fence() {
    let raw = "```json\n[{\"prompt\": \"Follow up?\"}]\n```";
    let suggestions = parse_suggestions(raw).expect("fenced json should still parse");
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].prompt, "Follow up?");
}

#[test]
fn strips_a_bare_fence_with_no_json_tag() {
    let raw = "```\n[{\"prompt\": \"Follow up?\"}]\n```";
    let suggestions = parse_suggestions(raw).expect("fenced json should still parse");
    assert_eq!(suggestions.len(), 1);
}

#[test]
fn drops_malformed_json_entirely() {
    assert!(parse_suggestions("not json at all").is_none());
    assert!(parse_suggestions("{\"not\": \"an array\"}").is_none());
    assert!(parse_suggestions("[{\"missing_prompt_field\": true}]").is_none());
}

#[test]
fn accepts_an_explicit_empty_array() {
    let suggestions = parse_suggestions("[]").expect("empty array is valid");
    assert!(suggestions.is_empty());
}

#[test]
fn drops_blank_prompt_entries_and_caps_at_max() {
    let raw = r#"[
        {"prompt": "  "},
        {"prompt": "one"},
        {"prompt": "two"},
        {"prompt": "three"},
        {"prompt": "four"}
    ]"#;
    let suggestions = parse_suggestions(raw).unwrap();
    assert_eq!(suggestions.len(), MAX_SUGGESTIONS);
    assert_eq!(suggestions[0].prompt, "one");
}

#[test]
fn trims_whitespace_from_prompt_and_label() {
    let raw = r#"[{"prompt": "  spaced  ", "label": "  Label  "}]"#;
    let suggestions = parse_suggestions(raw).unwrap();
    assert_eq!(suggestions[0].prompt, "spaced");
    assert_eq!(suggestions[0].label.as_deref(), Some("Label"));
}

#[test]
fn empty_label_string_becomes_none() {
    let raw = r#"[{"prompt": "x", "label": "   "}]"#;
    let suggestions = parse_suggestions(raw).unwrap();
    assert_eq!(suggestions[0].label, None);
}

// ── generate_and_emit: end-to-end against a scripted model ─────────────────

struct ScriptedTextModel {
    text: String,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl ChatModel<()> for ScriptedTextModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ModelResponse::assistant(self.text.clone()))
    }
}

async fn drain_suggestions_event(
    rx: &mut tokio::sync::broadcast::Receiver<WebChannelEvent>,
    thread_id: &str,
) -> Option<WebChannelEvent> {
    loop {
        match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
            Ok(Ok(ev)) if ev.event == "chat_suggestions" && ev.thread_id == thread_id => {
                return Some(ev)
            }
            Ok(Ok(_)) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            _ => return None,
        }
    }
}

#[tokio::test]
async fn emits_chat_suggestions_for_a_well_formed_reply() {
    let _override = test_provider_override::install_model(Arc::new(ScriptedTextModel {
        text: r#"[{"prompt": "What's next?", "label": "Next steps"}]"#.to_string(),
        calls: Arc::new(AtomicUsize::new(0)),
    }));
    let mut rx = subscribe_web_channel_events();

    let thread_id = "sugg-thread-ok";
    generate_and_emit(
        &crate::core::runtime::DispatchCtx::ambient(),
        "client-1",
        thread_id,
        "req-1",
        "How do I deploy this?",
        "Run `pnpm build` then `pnpm deploy`.",
    )
    .await;

    let ev = drain_suggestions_event(&mut rx, thread_id)
        .await
        .expect("chat_suggestions should have been emitted");
    assert_eq!(ev.turn_request_id, Some("req-1".to_string()));
    let suggestions = ev.suggestions.expect("suggestions payload");
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].prompt, "What's next?");
}

#[tokio::test]
async fn drops_silently_when_the_model_returns_garbage() {
    let _override = test_provider_override::install_model(Arc::new(ScriptedTextModel {
        text: "I cannot comply with strict JSON today.".to_string(),
        calls: Arc::new(AtomicUsize::new(0)),
    }));
    let mut rx = subscribe_web_channel_events();

    let thread_id = "sugg-thread-garbage";
    generate_and_emit(
        &crate::core::runtime::DispatchCtx::ambient(),
        "client-1",
        thread_id,
        "req-2",
        "How do I deploy this?",
        "Run `pnpm build` then `pnpm deploy`.",
    )
    .await;

    assert!(
        drain_suggestions_event(&mut rx, thread_id).await.is_none(),
        "malformed model output must never surface as chat_suggestions"
    );
}

#[tokio::test]
async fn skips_when_the_user_message_is_too_short() {
    let calls = Arc::new(AtomicUsize::new(0));
    let _override = test_provider_override::install_model(Arc::new(ScriptedTextModel {
        text: r#"[{"prompt": "x"}]"#.to_string(),
        calls: calls.clone(),
    }));

    generate_and_emit(
        &crate::core::runtime::DispatchCtx::ambient(),
        "client-1",
        "sugg-thread-short",
        "req-3",
        "ok",
        "Sure thing!",
    )
    .await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the model must not be called for a trivial user message"
    );
}

#[test]
fn suggestions_request_asks_for_no_reasoning() {
    let request = build_suggestions_request("what is the weather in Paris", "Sunny, 21°C.");
    assert_eq!(
        request.metadata.get("openhuman_reasoning_off"),
        Some(&serde_json::Value::Bool(true)),
        "follow-up suggestions must opt out of reasoning"
    );
    // The hint is metadata only; nothing provider-specific is set here, so a
    // BYOK or local provider receives an unchanged body.
    assert!(request.provider_options.is_null());
}
