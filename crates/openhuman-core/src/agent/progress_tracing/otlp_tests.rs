use super::*;
use std::collections::BTreeMap;

fn span(id: &str, parent: Option<&str>, kind: SpanKind, name: &str) -> TraceSpan {
    TraceSpan {
        trace_id: "thread-one:turn-one".into(),
        span_id: id.into(),
        parent_span_id: parent.map(str::to_string),
        name: name.into(),
        kind,
        start_unix_ms: 1_000,
        end_unix_ms: Some(2_000),
        status: SpanStatus::Ok,
        attributes: BTreeMap::new(),
        input: None,
        output: None,
    }
}

fn attr<'a>(span: &'a Value, key: &str) -> Option<&'a str> {
    span["attributes"]
        .as_array()?
        .iter()
        .find(|item| item["key"] == key)?["value"]["stringValue"]
        .as_str()
}

#[test]
fn otlp_has_one_root_with_readable_io_and_call_only_usage() {
    let mut root = span("root", None, SpanKind::Turn, "agent.turn:orchestrator");
    root.attributes
        .insert("thread.id".into(), json!("thread-one"));
    root.attributes
        .insert("run.type".into(), json!("interactive_chat"));
    root.attributes
        .insert("gen_ai.usage.input_tokens".into(), json!(20));
    root.attributes
        .insert("gen_ai.usage.output_tokens".into(), json!(5));
    root.input = Some(json!("What is the weather?"));
    root.output = Some(json!("Sunny."));
    let mut generation = span("call", Some("root"), SpanKind::Generation, "llm.reply");
    generation.attributes.insert(
        "gen_ai.request.model".into(),
        json!("managed.openrouter/test"),
    );
    generation
        .attributes
        .insert("gen_ai.usage.input_tokens".into(), json!(20));
    generation
        .attributes
        .insert("gen_ai.usage.cached_input_tokens".into(), json!(8));
    generation
        .attributes
        .insert("gen_ai.usage.output_tokens".into(), json!(5));
    generation.input = Some(json!([{ "role": "user", "content": "What is the weather?" }]));
    generation.output = Some(json!({ "content": "Sunny." }));
    let payloads = otlp_requests(&[root, generation], "production");
    assert_eq!(payloads.len(), 1);
    let spans = payloads[0]["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .unwrap();
    assert_eq!(spans.len(), 2);
    let root = spans
        .iter()
        .find(|item| item["name"] == "agent.turn:orchestrator")
        .unwrap();
    let call = spans
        .iter()
        .find(|item| item["name"] == "llm.reply")
        .unwrap();
    assert_eq!(attr(root, "langfuse.observation.type"), Some("agent"));
    assert_eq!(attr(root, "langfuse.session.id"), Some("thread-one"));
    assert_eq!(
        attr(root, "langfuse.observation.input"),
        Some("\"What is the weather?\"")
    );
    assert_eq!(
        attr(root, "langfuse.observation.output"),
        Some("\"Sunny.\"")
    );
    assert!(attr(root, "langfuse.observation.usage_details").is_none());
    assert_eq!(call["parentSpanId"], root["spanId"]);
    assert_eq!(attr(call, "langfuse.observation.type"), Some("generation"));
    assert_eq!(
        attr(call, "langfuse.observation.model.name"),
        Some("managed.openrouter/test")
    );
    assert_eq!(
        attr(call, "langfuse.observation.usage_details"),
        Some("{\"cache_read_input_tokens\":8,\"input\":12,\"output\":5,\"total\":25}")
    );
}

#[test]
fn repeated_internal_searches_become_one_summary() {
    let root = span("root", None, SpanKind::Turn, "agent.turn");
    let searches: Vec<_> = (0..240)
        .map(|index| {
            span(
                &format!("search-{index}"),
                Some("root"),
                SpanKind::Tool,
                "tool.tool_search",
            )
        })
        .collect();
    let mut all = vec![root];
    all.extend(searches);
    let payloads = otlp_requests(&all, "production");
    let spans = payloads[0]["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .unwrap();
    assert_eq!(spans.len(), 2);
    let summary = spans
        .iter()
        .find(|item| item["name"] == "tool.search_catalog")
        .unwrap();
    assert_eq!(
        attr(summary, "langfuse.observation.metadata.tool_call_count"),
        Some("240")
    );
}

#[test]
fn tinyinference_messages_render_as_role_labeled_conversation() {
    let input = json!([
        { "system": { "content": [{ "text": "instructions" }] } },
        { "user": { "content": [{ "text": "find weather" }] } },
        { "assistant": { "content": [], "tool_calls": [{
            "id": "call-1", "name": "weather", "arguments": { "city": "Paris" }
        }] } },
        { "tool": { "tool_call_id": "call-1", "content": [{ "text": "sunny" }] } },
        { "custom": { "kind": "compaction" } },
    ]);
    let normalized = normalize_generation_io(&input, false);
    let messages = normalized.as_array().unwrap();
    assert_eq!(messages.len(), 4);
    assert_eq!(
        messages[0],
        json!({ "role": "system", "content": "instructions" })
    );
    assert_eq!(
        messages[1],
        json!({ "role": "user", "content": "find weather" })
    );
    assert_eq!(
        messages[2]["tool_calls"][0]["function"]["arguments"],
        "{\"city\":\"Paris\"}"
    );
    assert_eq!(
        messages[3],
        json!({ "role": "tool", "tool_call_id": "call-1", "content": "sunny" })
    );
    let output = normalize_generation_io(&json!({ "content": [{ "text": "It is sunny" }] }), true);
    assert_eq!(
        output,
        json!({ "role": "assistant", "content": "It is sunny" })
    );
}

#[test]
fn subagent_root_shows_task_and_final_answer() {
    let mut root = span("root", None, SpanKind::Turn, "agent.turn:researcher");
    root.input = Some(json!("unreadable serialized model request"));
    let mut first = span("first", Some("root"), SpanKind::Generation, "model");
    first.input = Some(json!([
        { "system": { "content": [{ "text": "instructions" }] } },
        { "user": { "content": [{ "text": "research topic" }] } },
    ]));
    first.output = Some(json!({ "content": [{ "text": "working" }] }));
    let mut last = span("last", Some("root"), SpanKind::Generation, "model");
    last.output = Some(json!({ "content": [{ "text": "final result" }] }));
    let mut spans = vec![root, first, last];
    prepare_subagent_root(&mut spans);
    assert_eq!(spans[0].input, Some(json!("research topic")));
    assert_eq!(spans[0].output, Some(json!("final result")));
}

#[test]
fn child_model_usage_is_exported_only_in_child_trace() {
    let root = span("root", None, SpanKind::Turn, "agent.turn");
    let child = span(
        "child",
        Some("root"),
        SpanKind::Subagent,
        "subagent.researcher",
    );
    let mut generation = span("model", Some("child"), SpanKind::Generation, "model");
    generation
        .attributes
        .insert("gen_ai.usage.input_tokens".into(), json!(100));
    generation
        .attributes
        .insert("gen_ai.usage.output_tokens".into(), json!(10));
    let payloads = otlp_requests(&[root, child, generation], "production");
    let spans = payloads[0]["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .unwrap();
    assert_eq!(spans.len(), 2);
    assert!(spans
        .iter()
        .all(|span| attr(span, "langfuse.observation.usage_details").is_none()));
}

#[tokio::test]
async fn external_backend_is_skipped_before_session_lookup() {
    let mut config = Config::default();
    config.api_url = Some("https://external.example.invalid/openai/v1".into());
    let spans = [span("root", None, SpanKind::Turn, "agent.turn")];
    assert!(push_spans(&config, &spans).await.is_ok());
}

#[test]
fn large_trace_is_split_under_backend_request_limit() {
    let mut spans = vec![span("root", None, SpanKind::Turn, "agent.turn")];
    for index in 0..18 {
        let mut tool = span(
            &format!("tool-{index}"),
            Some("root"),
            SpanKind::Tool,
            "tool.fetch",
        );
        tool.output = Some(json!("x".repeat(500_000)));
        spans.push(tool);
    }
    let requests = otlp_requests(&spans, "production");
    assert!(requests.len() >= 2);
    assert!(requests
        .iter()
        .all(|request| request.to_string().len() < MAX_REQUEST_BYTES));
}
