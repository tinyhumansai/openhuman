use super::*;

/// An unparseable base is the fail-closed default rather than a panic or a
/// pushable bucket. `ingestion_url` returns a non-URL placeholder when no
/// backend host resolves.
#[test]
fn an_unparseable_base_is_production() {
    for base in ["", "not a url", "/api/v1/ingestion"] {
        assert_eq!(
            environment_for_base(base),
            "production",
            "{base:?} must fail closed"
        );
    }
}

#[test]
fn trace_create_carries_environment_release_and_run_tags() {
    let mut turn = span(
        "trace-1",
        "root",
        None,
        "agent.turn",
        SpanKind::Turn,
        SpanStatus::Ok,
        1_000,
        Some(2_000),
    );
    turn.attributes
        .insert("run.type".into(), json!("autonomous_task"));
    turn.attributes
        .insert("channel.source".into(), json!("autonomous"));
    let payload = spans_to_langfuse_batch(&[turn], false, "staging");
    let trace = &payload["batch"][0]["body"];
    // Top-level Langfuse trace fields, not metadata.
    assert_eq!(trace["environment"], "staging");
    assert_eq!(trace["release"], env!("CARGO_PKG_VERSION"));
    // Filterable run tags + run_type metadata.
    assert_eq!(
        trace["tags"],
        json!(["run:autonomous_task", "source:autonomous"])
    );
    assert_eq!(trace["metadata"]["run_type"], "autonomous_task");
}

#[test]
fn interactive_chat_trace_gets_interactive_run_tag() {
    let mut turn = span(
        "trace-1",
        "root",
        None,
        "agent.turn",
        SpanKind::Turn,
        SpanStatus::Ok,
        1_000,
        Some(2_000),
    );
    turn.attributes
        .insert("run.type".into(), json!("interactive_chat"));
    turn.attributes
        .insert("channel.source".into(), json!("chat"));
    let payload = spans_to_langfuse_batch(&[turn], false, "production");
    let trace = &payload["batch"][0]["body"];
    assert_eq!(
        trace["tags"],
        json!(["run:interactive_chat", "source:chat"])
    );
    assert_eq!(trace["metadata"]["run_type"], "interactive_chat");
}

#[test]
fn generation_usage_details_map_reasoning_and_cache_tokens() {
    let mut gen = span(
        "trace-1",
        "gen-1",
        Some("root"),
        "llm.agentic-v1",
        SpanKind::Generation,
        SpanStatus::Ok,
        1_000,
        Some(1_500),
    );
    gen.attributes.clear();
    gen.attributes
        .insert("gen_ai.request.model".into(), json!("agentic-v1"));
    gen.attributes
        .insert("gen_ai.usage.input_tokens".into(), json!(1_000));
    gen.attributes
        .insert("gen_ai.usage.output_tokens".into(), json!(200));
    gen.attributes
        .insert("gen_ai.usage.cached_input_tokens".into(), json!(0));
    gen.attributes
        .insert("gen_ai.usage.reasoning_tokens".into(), json!(128));
    gen.attributes
        .insert("gen_ai.usage.cache_creation_tokens".into(), json!(64));
    gen.attributes
        .insert("gen_ai.usage.cost_usd".into(), json!(0.0042));
    gen.attributes
        .insert("gen_ai.provider".into(), json!("managed"));

    let payload = spans_to_langfuse_batch(&[gen], false, "production");
    let obs = &payload["batch"][1];
    assert_eq!(obs["type"], "generation-create");
    let usage = &obs["body"]["usageDetails"];
    assert_eq!(usage["input"], 1_000);
    assert_eq!(usage["output"], 200);
    // Cache reads always flow, even at 0.
    assert_eq!(usage["cache_read_input_tokens"], 0);
    assert_eq!(usage["reasoning_tokens"], 128);
    assert_eq!(usage["cache_creation_input_tokens"], 64);
    assert_eq!(obs["body"]["costDetails"]["total"], 0.0042);
    // Provenance rides in observation metadata.
    assert_eq!(obs["body"]["metadata"]["gen_ai.provider"], "managed");
}

#[test]
fn generation_without_reasoning_or_cache_write_omits_those_usage_keys() {
    let mut gen = span(
        "trace-1",
        "gen-1",
        Some("root"),
        "llm.agentic-v1",
        SpanKind::Generation,
        SpanStatus::Ok,
        1_000,
        Some(1_500),
    );
    gen.attributes.clear();
    gen.attributes
        .insert("gen_ai.usage.input_tokens".into(), json!(10));
    gen.attributes
        .insert("gen_ai.usage.output_tokens".into(), json!(5));
    let payload = spans_to_langfuse_batch(&[gen], false, "production");
    let usage = &payload["batch"][1]["body"]["usageDetails"];
    assert_eq!(
        usage["cache_read_input_tokens"], 0,
        "cache reads always present"
    );
    assert!(usage.get("reasoning_tokens").is_none());
    assert!(usage.get("cache_creation_input_tokens").is_none());
}

#[test]
fn error_span_gets_error_level_and_status_message() {
    let mut tool = span(
        "trace-1",
        "tool-1",
        Some("root"),
        "tool.shell",
        SpanKind::Tool,
        SpanStatus::Error,
        1_000,
        Some(1_200),
    );
    tool.attributes
        .insert("error.message".into(), json!("The command timed out"));
    let payload = spans_to_langfuse_batch(&[tool], false, "production");
    let obs = &payload["batch"][1]["body"];
    assert_eq!(obs["level"], "ERROR");
    assert_eq!(obs["statusMessage"], "The command timed out");

    // Without a captured message: ERROR level, no statusMessage.
    let bare = span(
        "trace-1",
        "tool-2",
        Some("root"),
        "tool.shell",
        SpanKind::Tool,
        SpanStatus::Error,
        1_000,
        Some(1_200),
    );
    let payload = spans_to_langfuse_batch(&[bare], false, "production");
    let obs = &payload["batch"][1]["body"];
    assert_eq!(obs["level"], "ERROR");
    assert!(obs.get("statusMessage").is_none());
}

#[tokio::test]
async fn empty_spans_push_is_ok_noop() {
    let config = Config::default();
    // Empty batch short-circuits before any host/token resolution or network.
    assert!(push_spans(&config, &[]).await.is_ok());
}
