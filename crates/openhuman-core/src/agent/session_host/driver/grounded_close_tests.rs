use super::*;
use std::sync::Arc;

const RECORDS: &str = "\n- `list_directory` — ok\n  > three crates\n";
const CLEAN_REPLY: &str = "The workspace holds three crates; the build is green.";

fn classified_halt(note: &str, class: &str) -> TinyagentsTurnOutcome {
    TinyagentsTurnOutcome {
        deferred_terminal: None,
        text: String::new(),
        resolved_route: None,
        history: Vec::new(),
        conversation: Vec::new(),
        model_calls: 1,
        tool_calls: 1,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        cost: tinyagents_harness::cost::CostTally::default(),
        early_exit_tool: None,
        hit_cap: false,
        wrap_up_injected: false,
        truncated: false,
        breaker_halt: Some(note.into()),
        failure_cause: Some(tinyagents_harness::no_progress::ClassifiedStopCause {
            class: class.into(),
            operation: "search".into(),
            summary: note.into(),
        }),
        tool_outcomes: vec![],
        compaction: None,
    }
}

#[tokio::test]
async fn classified_halt_returns_partial_work_without_provider_usage() {
    crate::agent::session_host::test_support::with_native_security(async {

    use tinyagents_harness::turn_outcome::ToolCallOutcome;
    let outcome = TinyagentsTurnOutcome {
        deferred_terminal: None,
        text: String::new(),
        resolved_route: None,
        history: Vec::new(),
        conversation: Vec::new(),
        model_calls: 2,
        tool_calls: 2,
        input_tokens: 100,
        output_tokens: 20,
        cached_input_tokens: 0,
        cost: tinyagents_harness::cost::CostTally::default(),
        early_exit_tool: None,
        hit_cap: false,
        wrap_up_injected: false,
        truncated: false,
        breaker_halt: Some("Stopping after 1 attempt(s): failure class `permission` still blocks operation `search` on `catalog`. Resolve this blocker before retrying.".into()),
        failure_cause: Some(tinyagents_harness::no_progress::ClassifiedStopCause {
            class: "permission".into(),
            operation: "search".into(),
            summary: "Stopping after 1 attempt(s): failure class `permission` still blocks operation `search` on `catalog`. Resolve this blocker before retrying.".into(),
        }),
        tool_outcomes: vec![
            ToolCallOutcome { call_id: "a".into(), name: "list".into(), arguments: serde_json::json!({}), success: true, content: "three items".into(), duration_ms: 1 },
            ToolCallOutcome { call_id: "b".into(), name: "search".into(), arguments: serde_json::json!({}), success: false, content: "403 Forbidden".into(), duration_ms: 1 },
        ],
        compaction: None,
    };
    let close = classified_halt_close(&outcome).expect("classified halt");
    assert_eq!(close.usage.model_calls, 0);
    assert!(close.output.contains("permission"), "{}", close.output);
    assert!(
        close.output.contains("`list` succeeded"),
        "{}",
        close.output
    );
    assert!(
        !close.output.contains("three items"),
        "no raw tool output: {}",
        close.output
    );
    assert!(close.output.contains(if cfg!(feature = "security-module") { "403 Forbidden" } else { "[REDACTED]" }), "{}", close.output);
    }).await;
}

/// The host adapter delegates generation and verification to TinyAgents while
/// retaining its selected model, temperature, output cap and billing gauge.
#[tokio::test]
async fn grounded_close_uses_the_selected_model_and_keeps_verifier_out_of_context_gauge() {
    use tinyagents_harness::testkit::ScriptedModel;
    use tinyinference_llm::message::{
        AssistantMessage, ContentBlock, ImageRef, Message, ToolMessage, UserMessage,
    };
    use tinyinference_llm::tool::ToolCall;
    use tinyinference_llm::usage::Usage;

    let generation = tinyinference_llm::model::ModelResponse::assistant(CLEAN_REPLY)
        .with_usage(Usage::new(90, 12));
    let verification =
        tinyinference_llm::model::ModelResponse::assistant("ACCEPT").with_usage(Usage::new(7, 2));
    let scripted = Arc::new(ScriptedModel::new(vec![generation, verification]));
    let source = TurnModelSource::from_model(scripted.clone());
    let history = vec![
        Message::User(UserMessage {
            content: vec![
                ContentBlock::Text("inspect ".into()),
                ContentBlock::Image(ImageRef {
                    url: "data:image/png;base64,AAAA".into(),
                    mime_type: Some("image/png".into()),
                }),
            ],
        }),
        Message::Assistant(AssistantMessage {
            id: Some("assistant-1".into()),
            content: vec![
                ContentBlock::thinking("reasoning context"),
                ContentBlock::Text("calling the tool".into()),
            ],
            tool_calls: vec![ToolCall {
                id: "tool-call-1".into(),
                name: "list_directory".into(),
                arguments: serde_json::json!({"path": "."}),
                invalid: None,
            }],
            usage: None,
            origin: None,
        }),
        Message::Tool(ToolMessage {
            tool_call_id: "tool-call-1".into(),
            content: vec![ContentBlock::Text("three crates".into())],
            trusted_verbatim: false,
            artifact: None,
        }),
    ];
    let mut outcome = blank_outcome(false);
    outcome.tool_outcomes = vec![tinyagents_harness::turn_outcome::ToolCallOutcome {
        call_id: "call-1".into(),
        name: "list_directory".into(),
        arguments: serde_json::Value::Null,
        success: true,
        content: "three crates".into(),
        duration_ms: 0,
    }];

    let closed = close_if_needed(
        &source,
        "host-selected-model",
        0.37,
        None,
        &tinytools_agent::dialect::NativeDialect,
        &history,
        "Summarize the workspace.",
        &outcome,
        None,
        None,
    )
    .await
    .expect("blank terminal reply needs a close");

    assert_eq!(closed.output, CLEAN_REPLY);
    assert_eq!(closed.usage.model_calls, 2);
    assert_eq!(closed.usage.input_tokens, 97);
    assert_eq!(closed.usage.output_tokens, 14);
    assert_eq!(closed.usage.last_call_input_tokens, 90);
    assert_eq!(closed.usage.last_call_output_tokens, 12);
    let requests = scripted.requests();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request.model.as_deref(), Some("host-selected-model"));
        assert_eq!(request.temperature, Some(0.37));
        assert_eq!(request.max_tokens, Some(AGENT_TURN_MAX_OUTPUT_TOKENS));
        assert!(request.tools.is_empty());
    }
    let expected_history: Vec<_> = history
        .iter()
        .filter_map(tinyagents_runtime::transcript::message_to_native_transcript)
        .map(|message| crate::agent::tinyagents::chat_message_to_message(&message))
        .collect();
    assert_eq!(
        &requests[0].messages[..expected_history.len()],
        expected_history,
        "close requests preserve the transcript roundtrip for attachments, reasoning and tool rows"
    );
}

#[tokio::test]
async fn terminal_failed_close_requests_are_counted_without_usage() {
    use tinyagents_harness::testkit::ScriptedModel;

    let source = TurnModelSource::from_model(Arc::new(ScriptedModel::new(Vec::new())));
    let closed = close_if_needed(
        &source,
        "test-model",
        0.0,
        None,
        &tinytools_agent::dialect::NativeDialect,
        &[],
        "Summarize the workspace.",
        &blank_outcome(false),
        None,
        None,
    )
    .await
    .expect("blank terminal reply needs a close");

    assert_eq!(closed.usage.model_calls, 2, "one failed ask and its repair");
    assert_eq!(closed.usage.input_tokens, 0);
    assert_eq!(closed.usage.output_tokens, 0);
    assert_eq!(closed.usage.last_call_input_tokens, 0);
}

#[tokio::test]
async fn close_state_machine_calls_emit_correlated_native_model_events() {
    use std::sync::Mutex;
    use tinyagents_harness::events::{AgentEvent, EventListener, EventSink};
    use tinyagents_harness::ids::RunId;
    use tinyagents_harness::testkit::ScriptedModel;
    use tinyinference_llm::model::ModelResponse;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<AgentEvent>>);
    impl EventListener for Recorder {
        fn on_event(&self, record: &tinyagents_harness::events::EventRecord) {
            self.0.lock().unwrap().push(record.event.clone());
        }
    }

    async fn run(replies: Vec<ModelResponse>, expected_calls: usize, expect_failure: bool) {
        let run_id = RunId::new("close-state-machine-trace");
        let sink = EventSink::with_stream_id(run_id.as_str());
        let recorder = Arc::new(Recorder::default());
        sink.subscribe(recorder.clone());
        let trace = SupplementalTrace::new(sink, run_id.clone());
        let scripted = Arc::new(ScriptedModel::new(replies));
        let source = TurnModelSource::from_model(scripted.clone());

        close_if_needed(
            &source,
            "host-selected-model",
            0.25,
            None,
            &tinytools_agent::dialect::NativeDialect,
            &[],
            "Summarize the workspace.",
            &blank_outcome(false),
            None,
            Some(&trace),
        )
        .await
        .expect("blank terminal reply needs a close");

        let requests = scripted.requests();
        assert_eq!(requests.len(), expected_calls);
        let records = recorder.0.lock().unwrap();
        let started: Vec<_> = records
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ModelStarted { call_id, model } => {
                    assert_eq!(model, "host-selected-model");
                    Some(call_id.as_str())
                }
                _ => None,
            })
            .collect();
        let completed: Vec<_> = records
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ModelCompleted { call_id, .. } => Some(call_id.as_str()),
                _ => None,
            })
            .collect();
        let failed: Vec<_> = records
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ModelFailed { call_id, .. } => Some(call_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(started.len(), expected_calls);
        if expect_failure {
            assert!(completed.is_empty());
            assert_eq!(
                failed, started,
                "each failed request has one terminal event"
            );
        } else {
            assert_eq!(
                completed, started,
                "each completed request has one terminal event"
            );
            assert!(failed.is_empty());
        }
        for (request, call_id) in requests.iter().zip(started) {
            let correlation = request
                .correlation
                .as_ref()
                .expect("host model forwards native call correlation");
            assert_eq!(correlation.run_id, run_id.as_str());
            assert_eq!(correlation.call_id, call_id);
        }
    }

    run(
        vec![
            ModelResponse::assistant(CLEAN_REPLY),
            ModelResponse::assistant("ACCEPT"),
        ],
        2,
        false,
    )
    .await;
    run(
        vec![
            ModelResponse::assistant(CLEAN_REPLY),
            ModelResponse::assistant("REJECT"),
            ModelResponse::assistant(CLEAN_REPLY),
            ModelResponse::assistant("ACCEPT"),
        ],
        4,
        false,
    )
    .await;
    run(Vec::new(), 2, true).await;
}

#[tokio::test(start_paused = true)]
async fn close_stream_eof_and_timeout_emit_terminal_traces_without_timeout_usage() {
    use std::sync::Mutex;
    use tinyagents_harness::events::{AgentEvent, EventListener, EventSink};
    use tinyagents_harness::ids::RunId;
    use tinyinference_llm::model::{
        ChatModel, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
    };

    struct EmptyOrPendingModel(bool);
    #[async_trait::async_trait]
    impl ChatModel<()> for EmptyOrPendingModel {
        async fn invoke(
            &self,
            _state: &(),
            _request: ModelRequest,
        ) -> tinyinference_llm::Result<ModelResponse> {
            Err(tinyinference_llm::Error::Model("stream only".into()))
        }

        async fn stream(
            &self,
            _state: &(),
            _request: ModelRequest,
        ) -> tinyinference_llm::Result<ModelStream> {
            let items: std::pin::Pin<Box<dyn futures::Stream<Item = ModelStreamItem> + Send>> =
                if self.0 {
                    Box::pin(futures::stream::pending())
                } else {
                    Box::pin(futures::stream::empty())
                };
            Ok(ModelStream::new(items))
        }
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<AgentEvent>>);
    impl EventListener for Recorder {
        fn on_event(&self, record: &tinyagents_harness::events::EventRecord) {
            self.0.lock().unwrap().push(record.event.clone());
        }
    }

    async fn run(pending: bool) -> (usize, usize, usize) {
        let run_id = RunId::new(if pending {
            "close-timeout"
        } else {
            "close-eof"
        });
        let sink = EventSink::with_stream_id(run_id.as_str());
        let recorder = Arc::new(Recorder::default());
        sink.subscribe(recorder.clone());
        let trace = SupplementalTrace::new(sink, run_id);
        let source = TurnModelSource::from_model(Arc::new(EmptyOrPendingModel(pending)));
        let close = close_if_needed(
            &source,
            "host-selected-model",
            0.0,
            None,
            &tinytools_agent::dialect::NativeDialect,
            &[],
            "Summarize the workspace.",
            &blank_outcome(false),
            pending.then_some(std::time::Duration::from_secs(1)),
            Some(&trace),
        )
        .await
        .expect("blank terminal reply needs a close");
        let records = recorder.0.lock().unwrap();
        let started = records
            .iter()
            .filter(|event| matches!(event, AgentEvent::ModelStarted { .. }))
            .count();
        let completed = records
            .iter()
            .filter(|event| matches!(event, AgentEvent::ModelCompleted { .. }))
            .count();
        let failed = records
            .iter()
            .filter(|event| matches!(event, AgentEvent::ModelFailed { .. }))
            .count();
        if pending {
            assert!(!close.output.is_empty(), "timeout ships a fallback");
            assert_eq!(
                close.usage.model_calls, 0,
                "timeout has no completion callback"
            );
            assert_eq!((started, completed, failed), (1, 0, 1));
        } else {
            assert_eq!(
                close.usage.model_calls, 2,
                "each EOF retry is observed once"
            );
            assert_eq!((started, completed, failed), (2, 0, 2));
        }
        (started, completed, failed)
    }

    run(false).await;
    run(true).await;
}

#[tokio::test]
async fn required_output_repair_preserves_all_host_output_placements() {
    use tinyagents_harness::testkit::ScriptedModel;

    async fn repair(candidate: &str, reply: &str, streamed: bool) -> (String, Vec<String>) {
        let source = TurnModelSource::from_model(Arc::new(ScriptedModel::replies(vec![candidate])));
        let contract = tinyagents_harness::config::RequiredOutput {
            block_key: "result".into(),
            required_keys: vec!["status".into()],
        };
        let mut usage = RepairUsage::for_model("test-model");
        let (progress, mut receiver) = tokio::sync::mpsc::channel(2);
        let output = super::repair_required_output(
            &source,
            "test-model",
            0.0,
            None,
            &tinytools_agent::dialect::NativeDialect,
            &contract,
            &[],
            reply,
            streamed,
            Some(&progress),
            1,
            &mut usage,
            None,
        )
        .await
        .expect("reply lacks the required block");
        let mut deltas = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            if let crate::progress::AgentProgress::TextDelta { delta, .. } = event {
                deltas.push(delta);
            }
        }
        (output, deltas)
    }

    let valid = r#"{"result":"ok","status":"ok"}"#;
    let invalid = "not structured output";
    let reply = "Original reply.";
    let block = r#"{"result":"","status":""}"#;

    let (valid_unstreamed, valid_unstreamed_deltas) = repair(valid, reply, false).await;
    assert_eq!(valid_unstreamed, valid);
    assert!(valid_unstreamed_deltas.is_empty());
    let (valid_streamed, valid_streamed_deltas) = repair(valid, reply, true).await;
    assert_eq!(valid_streamed, format!("{reply}\n\n{valid}"));
    assert_eq!(valid_streamed_deltas, vec![format!("\n\n{valid}")]);
    let (invalid_streamed, invalid_streamed_deltas) = repair(invalid, reply, true).await;
    assert_eq!(invalid_streamed, format!("{reply}\n\n{block}"));
    assert_eq!(invalid_streamed_deltas, vec![format!("\n\n{block}")]);
    let (invalid_unstreamed, invalid_unstreamed_deltas) = repair(invalid, reply, false).await;
    assert_eq!(invalid_unstreamed, format!("{block}\n\n{reply}"));
    assert!(invalid_unstreamed_deltas.is_empty());
}

fn blank_outcome(truncated: bool) -> TinyagentsTurnOutcome {
    TinyagentsTurnOutcome {
        deferred_terminal: None,
        text: String::new(),
        resolved_route: None,
        history: Vec::new(),
        conversation: Vec::new(),
        model_calls: 3,
        tool_calls: 1,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        cost: tinyagents_harness::cost::CostTally::default(),
        early_exit_tool: None,
        hit_cap: false,
        wrap_up_injected: false,
        breaker_halt: None,
        failure_cause: None,
        truncated,
        compaction: None,
        tool_outcomes: Vec::new(),
    }
}

/// #6951: a turn that ended because the model ran out of output tokens while
/// reasoning is closed with that cause named, not with "you have finished
/// using tools" — which told the model its unfinished work was done.
#[test]
fn a_truncated_turn_is_closed_with_the_output_budget_named() {
    let truncated = close_instruction(&blank_outcome(true), false, RECORDS);
    assert!(
        truncated.contains("ran out of output tokens"),
        "{truncated}"
    );
    assert!(
        !truncated.contains("You have finished using tools"),
        "{truncated}"
    );
    assert!(truncated.contains("<tool_records>"), "{truncated}");

    let finished = close_instruction(&blank_outcome(false), false, RECORDS);
    assert!(
        finished.contains("You have finished using tools"),
        "{finished}"
    );
}

/// The cap checkpoint still wins when the run also hit its call cap.
#[test]
fn a_capped_truncated_turn_gets_the_cap_checkpoint() {
    let out = close_instruction(&blank_outcome(true), true, RECORDS);
    assert!(!out.contains("ran out of output tokens"), "{out}");
    assert!(out.contains("<tool_records>"), "{out}");
}

/// The same for the required-output repair (#7721 review): its call answered,
/// then streaming the correction outlived the turn budget. The caller-owned
/// accumulator still holds the call.
#[tokio::test(start_paused = true)]
async fn timed_out_repair_keeps_the_usage_of_its_answered_call() {
    use crate::progress::AgentProgress;
    let source = crate::agent::tinyagents::TurnModelSource::from_model(std::sync::Arc::new(
        tinyagents_harness::testkit::ScriptedModel::replies(vec!["no block"]),
    ));
    let contract = tinyagents_harness::config::RequiredOutput {
        block_key: "result".into(),
        required_keys: vec!["status".into()],
    };
    // A full channel whose receiver is alive: the correction's send blocks.
    let (progress, _receiver) = tokio::sync::mpsc::channel(1);
    assert!(progress
        .try_send(AgentProgress::TextDelta {
            delta: "earlier".into(),
            iteration: 1,
        })
        .is_ok());
    let mut usage = RepairUsage::for_model("test-model");

    let repaired = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        repair_required_output(
            &source,
            "test-model",
            0.0,
            None,
            &tinytools_agent::dialect::NativeDialect,
            &contract,
            &[],
            "Done.",
            true,
            Some(&progress),
            2,
            &mut usage,
            None,
        ),
    )
    .await;

    assert!(repaired.is_err(), "the blocked send outlives the budget");
    assert_eq!(usage.model_calls, 1, "the answered repair call is recorded");
}

#[test]
fn supplemental_cached_response_is_marked_before_usage_without_provider_charge() {
    use std::sync::{Arc, Mutex};
    use tinyagents_harness::events::{AgentEvent, EventListener, EventSink};
    use tinyagents_harness::ids::RunId;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<AgentEvent>>);
    impl EventListener for Recorder {
        fn on_event(&self, record: &tinyagents_harness::events::EventRecord) {
            self.0.lock().unwrap().push(record.event.clone());
        }
    }

    let run_id = RunId::new("host-supplemental-cache-fixture");
    let sink = EventSink::with_stream_id(run_id.as_str());
    let recorder = Arc::new(Recorder::default());
    sink.subscribe(recorder.clone());
    let trace = SupplementalTrace::new(sink, run_id);
    let (call_id, started_at_ms) = trace.begin("gpt-4.1");
    let usage = tinyinference_llm::usage::Usage {
        input_tokens: 14,
        output_tokens: 3,
        total_tokens: 17,
        charged_amount: Some(tinyinference_llm::usage::ChargedAmount::usd_micros(900)),
        ..Default::default()
    };
    let mut response = tinyinference_llm::model::ModelResponse::assistant("cached answer");
    response.served_from_cache = true;
    response.usage = Some(usage);
    response.message.usage = Some(usage);
    trace.completed(
        call_id,
        "gpt-4.1",
        started_at_ms,
        serde_json::json!(["cached prompt"]),
        &response,
    );

    let records = recorder.0.lock().unwrap();
    assert!(matches!(records.as_slice(), [
        AgentEvent::ModelStarted { .. },
        AgentEvent::ModelResponseCached { .. },
        AgentEvent::UsageRecorded { usage },
        AgentEvent::ModelCompleted { usage: Some(completed), .. },
        AgentEvent::GenerationEnriched { enrichment, .. },
    ] if usage.charged_amount.is_none()
        && completed.charged_amount.is_none()
        && enrichment.estimated_cost_usd.is_none()));
}

#[test]
fn supplemental_free_route_uses_zero_catalog_rates() {
    use std::sync::{Arc, Mutex};
    use tinyagents_harness::events::{AgentEvent, EventListener, EventSink};
    use tinyagents_harness::ids::RunId;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<AgentEvent>>);
    impl EventListener for Recorder {
        fn on_event(&self, record: &tinyagents_harness::events::EventRecord) {
            self.0.lock().unwrap().push(record.event.clone());
        }
    }

    let run_id = RunId::new("host-supplemental-free-fixture");
    let sink = EventSink::with_stream_id(run_id.as_str());
    let recorder = Arc::new(Recorder::default());
    sink.subscribe(recorder.clone());
    let trace = SupplementalTrace::new(sink, run_id);
    let (call_id, started_at_ms) = trace.begin("openrouter/test-model:free");
    let response = tinyinference_llm::model::ModelResponse::assistant("free answer");
    trace.completed(
        call_id,
        "openrouter/test-model:free",
        started_at_ms,
        serde_json::json!(["free prompt"]),
        &response,
    );

    let records = recorder.0.lock().unwrap();
    let enrichment = records
        .iter()
        .find_map(|event| match event {
            AgentEvent::GenerationEnriched { enrichment, .. } => Some(enrichment),
            _ => None,
        })
        .expect("supplemental generation carries catalog enrichment");
    let pricing = enrichment
        .pricing
        .as_ref()
        .expect("free routes have known zero rates");
    assert_eq!(pricing.input_per_mtok_usd, Some(0.0));
    assert_eq!(pricing.cached_input_per_mtok_usd, Some(0.0));
    assert_eq!(pricing.output_per_mtok_usd, Some(0.0));
    assert_eq!(enrichment.estimated_cost_usd, None);
}

#[path = "grounded_close_typed_stop_tests.rs"]
mod typed_stop;
