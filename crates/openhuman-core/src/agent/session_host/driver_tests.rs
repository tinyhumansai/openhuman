use super::*;

fn sidecar(
) -> std::sync::Arc<std::sync::Mutex<crate::agent::tinyagents::host::run_context::SessionTurnSidecar>>
{
    crate::agent::tinyagents::host::OpenHumanRunContext::new()
        .session_sidecar
        .clone()
}

#[test]
fn graph_failure_persists_only_accepted_snapshot_history() {
    let snapshot = Arc::new(std::sync::Mutex::new(
        crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshot {
            messages: vec![
                Message::system("stable prefix"),
                Message::user("request"),
                Message::assistant("provider reply not yet accepted by a later request"),
            ],
            accepted_len: 2,
            request_base_len: 2,
            ..Default::default()
        },
    ));
    let failure = driver_error_with_snapshot(
        anyhow::anyhow!("provider rejected follow-up"),
        &snapshot,
        &sidecar(),
        std::time::Duration::from_millis(1),
    );
    let partial = failure.partial.expect("snapshot should produce a partial");
    assert_eq!(partial.history.len(), 2);
    assert_eq!(partial.history[0].text(), "stable prefix");
    assert_eq!(partial.history[1].text(), "request");
    assert!(partial
        .partial
        .expect("display partial")
        .content
        .contains("provider rejected follow-up"));
}

#[test]
fn stalled_model_stream_reports_completed_evidence_instead_of_its_narration() {
    let snapshot = Arc::new(std::sync::Mutex::new(
        crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshot {
            messages: vec![Message::user("Find information about Jev")],
            accepted_len: 1,
            request_base_len: 1,
            tool_outcomes: vec![tinyagents_harness::turn_outcome::ToolCallOutcome {
                call_id: "search-1".into(),
                name: "web_search_tool".into(),
                arguments: serde_json::json!({"query": "Jev TypeSafe"}),
                success: true,
                content: "TypeSafe describes Jev as a System One model".into(),
                duration_ms: 12,
            }],
            ..Default::default()
        },
    ));
    let failure = driver_error_with_snapshot(
        anyhow::Error::new(tinyagents_harness::TinyAgentsError::GenerationStalled),
        &snapshot,
        &sidecar(),
        std::time::Duration::from_millis(1),
    );
    let terminal = failure.outcome.as_ref().expect("typed terminal outcome");
    assert_eq!(
        terminal.reason,
        tinyagents_harness::terminal::TerminalReason::ProviderFailed(Some(
            tinyagents_harness::retry::FailoverReason::classify(
                &tinyagents_harness::TinyAgentsError::GenerationStalled
            )
        ))
    );
    let partial = failure.partial.expect("interrupted partial");
    let display = partial.partial.expect("display partial").content;
    assert!(display.contains("stopped a repetitive model response"));
    assert!(display.contains("TypeSafe describes Jev as a System One model"));
    assert!(!display.contains("Let me"));
}

#[test]
fn graph_failure_copies_snapshot_usage_and_failed_tool_outcome_to_sidecar() {
    let snapshot = Arc::new(std::sync::Mutex::new(
        crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshot {
            messages: vec![Message::user("request"), Message::tool("call-1", "denied")],
            accepted_len: 1,
            request_base_len: 1,
            input_tokens: 21,
            output_tokens: 8,
            cached_input_tokens: 3,
            last_call_input_tokens: 13,
            last_call_output_tokens: 5,
            cost: tinyagents_harness::cost::CostTally {
                known_usd: 0.004,
                source: tinyagents_harness::cost::CostSource::Charged,
            },
            resolved_route: Some(tinyinference_llm::model::ResolvedModelRoute::new(
                "openhuman",
                "chat-concrete",
                "chat-v1",
            )),
            pricing_model: None,
            model_calls: 2,
            tool_outcomes: vec![tinyagents_harness::turn_outcome::ToolCallOutcome {
                call_id: "call-1".into(),
                name: "write_file".into(),
                arguments: serde_json::json!({"path": "blocked.txt"}),
                success: false,
                content: "denied".into(),
                duration_ms: 17,
            }],
        },
    ));
    let sidecar = sidecar();
    let failure = driver_error_with_snapshot(
        anyhow::anyhow!("tool follow-up was rejected"),
        &snapshot,
        &sidecar,
        std::time::Duration::from_millis(25),
    );
    let partial = failure.partial.expect("recoverable snapshot partial");
    assert_eq!(partial.history, vec![Message::user("request")]);
    let observed = sidecar.lock().expect("sidecar");
    assert_eq!(
        (
            observed.model_calls,
            observed.input_tokens,
            observed.output_tokens,
            observed.cached_input_tokens
        ),
        (2, 21, 8, 3)
    );
    // The final call's own size, not the two calls' sum, reaches the gauge.
    assert_eq!(
        (
            observed.last_call_input_tokens,
            observed.last_call_output_tokens
        ),
        (13, 5)
    );
    assert!((observed.cost.known_usd - 0.004).abs() < f64::EPSILON);
    assert_eq!(
        observed.cost.source,
        tinyagents_harness::cost::CostSource::Charged
    );
    let route = observed
        .resolved_route
        .as_ref()
        .expect("accepted route reaches sidecar");
    assert_eq!(
        (
            route.provider.as_str(),
            route.model.as_str(),
            route.route.as_str()
        ),
        ("openhuman", "chat-concrete", "chat-v1")
    );
    assert_eq!(observed.tool_calls, 1);
    assert_eq!(
        observed.duration,
        Some(std::time::Duration::from_millis(25))
    );
    assert_eq!(observed.tool_outcomes.len(), 1);
    let failure = &observed.tool_outcomes[0];
    assert!(
        !failure.success,
        "completed failure state survives graph error"
    );
    assert_eq!(failure.content, "denied");
    assert_eq!(
        failure.arguments,
        serde_json::json!({"path": "blocked.txt"})
    );
    assert_eq!(failure.duration_ms, 17);
}

#[test]
fn tool_snapshot_with_no_executable_source_fails_closed_before_graph() {
    let visible = HashSet::from(["revoked_tool".to_string()]);
    let error = ensure_snapshot_tools_are_executable(
        &visible,
        &Arc::new(Vec::new()),
        &Arc::new(Vec::new()),
        &HashSet::new(),
    )
    .expect_err("a declared tool must have a request-scoped executable source");
    assert!(error.error.to_string().contains("revoked_tool"));
}

#[test]
fn empty_snapshot_failure_still_carries_the_typed_terminal_outcome() {
    let snapshot = Arc::new(std::sync::Mutex::new(
        crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshot::default(),
    ));
    let typed = driver_error_with_snapshot(
        anyhow::Error::new(tinyagents_harness::TinyAgentsError::Cancelled),
        &snapshot,
        &sidecar(),
        std::time::Duration::from_millis(1),
    );
    assert!(typed.partial.is_none());
    let terminal = typed.outcome.expect("typed outcome on the empty branch");
    assert_eq!(
        terminal.reason,
        tinyagents_harness::terminal::TerminalReason::Cancelled
    );

    let untyped = driver_error_with_snapshot(
        anyhow::anyhow!("plain failure"),
        &snapshot,
        &sidecar(),
        std::time::Duration::from_millis(1),
    );
    assert!(untyped.outcome.is_none());
}

/// The context gauge reads the turn's final call. Repair runs after the
/// grounded close, which runs after the harness loop, so the newest of them
/// that reached the provider wins; a step that made no call is skipped.
#[test]
fn final_call_tokens_prefer_the_newest_call_that_reached_the_provider() {
    let call = |input: u64, output: u64| grounded_close::RepairUsage {
        model_calls: 1,
        last_call_input_tokens: input,
        last_call_output_tokens: output,
        ..Default::default()
    };
    let loop_last = Some((90_000, 400));

    assert_eq!(final_call_tokens(loop_last, None, None), (90_000, 400));
    assert_eq!(
        final_call_tokens(loop_last, Some(&call(95_000, 300)), None),
        (95_000, 300)
    );
    assert_eq!(
        final_call_tokens(loop_last, Some(&call(0, 300)), None),
        (0, 300)
    );
    assert_eq!(
        final_call_tokens(
            loop_last,
            Some(&call(95_000, 300)),
            Some(&call(96_000, 200))
        ),
        (96_000, 200)
    );
    // A close that recorded no provider usage did not make the final call.
    assert_eq!(
        final_call_tokens(
            loop_last,
            Some(&grounded_close::RepairUsage::default()),
            None
        ),
        (90_000, 400)
    );
    assert_eq!(final_call_tokens(None, None, None), (0, 0));
}

#[test]
fn recorded_declarations_allow_missing_executors_without_authorizing_fresh_names() {
    let historical = HashSet::from(["retired_tool".to_string()]);
    let empty = Arc::new(Vec::new());
    assert!(ensure_snapshot_tools_are_executable(&historical, &empty, &empty, &historical).is_ok());
    let mixed = HashSet::from(["retired_tool".to_string(), "invented_tool".to_string()]);
    let error = ensure_snapshot_tools_are_executable(&mixed, &empty, &empty, &historical)
        .expect_err("historical declarations never permit an unrecorded name");
    assert!(error.error.to_string().contains("invented_tool"));
    assert!(!error.error.to_string().contains("retired_tool"));
}

/// A Stop keeps the prompt and every round the provider accepted (#7715), and
/// nothing past the last answered request: not streamed text, and not a tool
/// call or result still in flight, which would replay as a dangling call.
#[test]
fn stopped_turn_hands_back_only_its_accepted_rounds() {
    let request = || vec![Message::user("earlier"), Message::user("stop me")];
    let cases: [(&str, Vec<Message>, usize, &[&str]); 3] = [
        (
            "mid-stream text",
            vec![
                Message::user("stop me"),
                Message::tool("call-1", "committed 6b75a4c"),
                Message::assistant("streamed, never accepted"),
            ],
            2,
            &["stop me", "committed 6b75a4c"],
        ),
        (
            "during a tool call",
            vec![
                Message::user("stop me"),
                Message::assistant("response that requested the tool"),
                Message::tool("call-2", "half-written output"),
            ],
            1,
            &["stop me"],
        ),
        // Before the first model call the snapshot is empty: keep the request.
        (
            "before the first model call",
            vec![],
            0,
            &["earlier", "stop me"],
        ),
    ];
    for (case, messages, accepted_len, expected) in cases {
        let snapshot = Arc::new(std::sync::Mutex::new(
            crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshot {
                messages,
                accepted_len,
                request_base_len: 1,
                model_calls: 2,
                ..Default::default()
            },
        ));
        let sidecar = sidecar();
        let failure = driver_cancelled_with_snapshot(
            &snapshot,
            &sidecar,
            std::time::Duration::from_millis(7),
            request(),
        );
        assert_eq!(failure.error, RuntimeError::Cancelled, "{case}");
        let partial = failure.partial.expect(case);
        let texts: Vec<String> = partial.history.iter().map(|m| m.text()).collect();
        assert_eq!(texts, expected, "{case}");
        assert!(partial.interrupted, "{case}");
        assert!(partial.partial.is_none(), "{case}: the mirror owns display");
        assert_eq!(sidecar.lock().unwrap().model_calls, 2, "{case}");
    }
}

/// A turn out of time skips the model repair (#7716); the required block is
/// then synthesized once and becomes the durable final row.
#[test]
fn synthesized_block_is_appended_once_and_replaces_the_final_row() {
    let contract = tinyagents_harness::config::RequiredOutput {
        block_key: "result".into(),
        required_keys: vec!["status".into()],
    };
    let mut output = "Done.".to_string();
    let mut history = vec![Message::user("go"), Message::assistant("Done.")];

    append_synthesized_block(&mut output, &mut history, &contract);
    assert!(tinyagents_harness::config::output_satisfies_contract(
        &output, &contract
    ));
    assert_eq!(
        history.len(),
        2,
        "the final row is replaced, not duplicated"
    );

    let once = output.clone();
    append_synthesized_block(&mut output, &mut history, &contract);
    assert_eq!(output, once, "a satisfied output is left alone");
}
