use super::*;
use tinyinference_llm::message::Message;

#[test]
fn with_thread_id_injects_explicit_thread() {
    let request = ModelRequest::new(vec![Message::user("hi")]);
    let updated = with_thread_id(request, Some("thread-42"));
    assert_eq!(
        updated.provider_options["thread_id"],
        serde_json::json!("thread-42")
    );
}

#[test]
fn an_explicit_reasoning_option_is_left_alone() {
    let request = ModelRequest::new(vec![Message::user("hi")])
        .with_provider_options(serde_json::json!({ "reasoning": { "effort": "high" } }));
    let updated = apply_reasoning_hint(request);
    assert_eq!(
        updated.provider_options["reasoning"],
        serde_json::json!({ "effort": "high" })
    );
}
