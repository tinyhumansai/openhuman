#[tokio::test]
async fn unknown_tool_call_projects_exactly_one_failed_timeline_row() {
    // Production Langfuse showed two ERROR tool observations ~15ms apart for
    // every unknown-tool call: the bridge synthesised a Started/Completed pair
    // off `UnknownToolCall` (null output, NotFound copy) and the crate's own
    // recovered pair produced a second one carrying the error text. Exactly
    // one row per call, carrying the error text and the NotFound class.
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());

    emit_recovered_unknown_tool_call(&sink, "c9");

    let mut started = Vec::new();
    let mut completed = Vec::new();
    while let Ok(p) = rx.try_recv() {
        match p {
            AgentProgress::ToolCallStarted {
                tool_name,
                display_label,
                ..
            } => started.push((tool_name, display_label)),
            AgentProgress::ToolCallCompleted {
                tool_name,
                success,
                failure,
                output,
                display_label,
                ..
            } => completed.push((tool_name, success, failure, output, display_label)),
            _ => {}
        }
    }
    assert_eq!(started.len(), 1, "exactly one started row: {started:?}");
    assert_eq!(completed.len(), 1, "exactly one completed row");
    assert_eq!(started[0].0, "search_files");
    assert_eq!(
        started[0].1.as_deref(),
        Some("Search Files (unavailable)"),
        "the timeline still says the tool was unavailable"
    );
    let (tool_name, success, failure, output, label) = completed.remove(0);
    assert_eq!(tool_name, "search_files");
    assert!(!success, "the attempted tool is projected as a failed call");
    assert_eq!(
        output, UNKNOWN_TOOL_ANSWER,
        "the row carries the error text"
    );
    assert_eq!(label.as_deref(), Some("Search Files (unavailable)"));
    // #6277: a tool the agent does not have fails identically on every retry,
    // so the timeline must not tell the user to "try again / run diagnostics".
    let failure = failure.expect("the failed row carries a classified failure");
    assert_eq!(
        failure.class,
        tinytools::status::ToolFailureClass::NotFound,
        "an unavailable tool must be classified NotFound, not Unknown"
    );
    assert!(!failure.recoverable);
}

#[tokio::test]
async fn unknown_tool_call_alone_projects_no_tool_row() {
    // The typed event only annotates the call; the crate's recovered pair is
    // what opens and closes the row.
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());

    sink.emit(AgentEvent::UnknownToolCall {
        call_id: "c9".into(),
        requested_name: "search_files".to_string(),
        arguments: serde_json::json!({}),
        recovery: "tool_error".to_string(),
    });
    while let Ok(p) = rx.try_recv() {
        assert!(
            !matches!(
                p,
                AgentProgress::ToolCallStarted { .. } | AgentProgress::ToolCallCompleted { .. }
            ),
            "UnknownToolCall must not synthesise its own tool row"
        );
    }
}

#[tokio::test]
async fn unknown_tool_call_in_a_child_run_projects_one_subagent_row() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::with_scope(
        Some(tx),
        "mock-model",
        "managed",
        10,
        Some(super::SubagentScope {
            agent_id: "researcher".to_string(),
            task_id: "task-1".to_string(),
            extended_policy: false,
        }),
        Arc::default(),
        Arc::default(),
        Arc::default(),
        Arc::default(),
        Vec::new(),
        crate::core::runtime::DispatchCtx::ambient(),
    );
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());

    emit_recovered_unknown_tool_call(&sink, "c10");

    let (mut started, mut completed) = (0, 0);
    let mut class = None;
    while let Ok(p) = rx.try_recv() {
        match p {
            AgentProgress::SubagentToolCallStarted { .. } => started += 1,
            AgentProgress::SubagentToolCallCompleted { failure, .. } => {
                completed += 1;
                class = failure.map(|f| f.class);
            }
            _ => {}
        }
    }
    assert_eq!((started, completed), (1, 1));
    assert_eq!(class, Some(tinytools::status::ToolFailureClass::NotFound));
}

/// W2-budget-dedupe: two `UsageRecorded` events for the *same* model call
/// (as happens once the observe-only crate `BudgetMiddleware` re-emits usage
/// its `after_model` folded, on top of the runtime's own emit) must be
/// recorded into the bridge accounting **exactly once**. Without the dedupe
/// guard the totals would double.
#[tokio::test]
async fn duplicate_usage_for_same_model_call_is_recorded_once() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());

    // One model call → one `ModelStarted` (iteration cursor → 1).
    sink.emit(AgentEvent::ModelStarted {
        call_id: "c1".into(),
        model: "mock-model".to_string(),
    });
    // Same call surfaces usage twice (runtime emit + BudgetMiddleware re-emit).
    sink.emit(AgentEvent::UsageRecorded {
        usage: Usage::new(100, 40),
    });
    sink.emit(AgentEvent::UsageRecorded {
        usage: Usage::new(100, 40),
    });

    // Totals reflect a single record, not two.
    let (input, output, _) = bridge.totals();
    assert_eq!(
        (input, output),
        (100, 40),
        "the duplicate UsageRecorded for the same iteration must be skipped"
    );

    // Exactly one `TurnCostUpdated` footer emit for the call.
    let mut cost_updates = 0;
    while let Ok(p) = rx.try_recv() {
        if matches!(p, AgentProgress::TurnCostUpdated { .. }) {
            cost_updates += 1;
        }
    }
    assert_eq!(cost_updates, 1, "footer must update once per model call");

    // A genuinely new model call (iteration cursor → 2) records again.
    sink.emit(AgentEvent::ModelStarted {
        call_id: "c2".into(),
        model: "mock-model".to_string(),
    });
    sink.emit(AgentEvent::UsageRecorded {
        usage: Usage::new(10, 5),
    });
    let (input, output, _) = bridge.totals();
    assert_eq!(
        (input, output),
        (110, 45),
        "a distinct model call (new iteration) must still record"
    );
}

// NOTE: the former `sentinel_tool_started_is_not_forwarded` test was removed
// here. The #4249 migration (commit 60097ba8d, "use sdk unknown tool
// recovery") deleted `UNKNOWN_TOOL_SENTINEL` + `UnknownToolRewriteMiddleware`
// in favour of the crate `UnknownToolPolicy::ReturnToolError` path, so a
// `ToolStarted` now only ever fires for real, model-visible tools (see the
// `ToolStarted` arm above — it no longer special-cases a sentinel). The test
// referenced the deleted constant (a stale reference reintroduced by a merge)
// and asserted behaviour that no longer exists.

/// #6XXX (tool-call presentation): `ToolCallStarted`/`ToolCallCompleted` must
/// carry the tool's OWN `display_label`/`display_detail` when the bridge was
/// built with the turn's tool sets, not a name-derived guess — proven with a
/// fake tool whose label is a fixed phrase and whose detail comes from a
/// `"to"` argument only known once the call completes.
#[tokio::test]
async fn tool_call_events_use_the_tool_s_own_display_label_and_detail() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::with_scope(
        Some(tx),
        "mock-model",
        "managed",
        10,
        None,
        Arc::default(),
        Arc::default(),
        Arc::default(),
        Arc::default(),
        fake_tool_sets(),
        crate::core::runtime::DispatchCtx::ambient(),
    );
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());

    sink.emit(AgentEvent::ModelStarted {
        call_id: "c1".into(),
        model: "mock-model".to_string(),
    });
    sink.emit(AgentEvent::ToolStarted {
        parent_call_id: None,
        call_id: "c1".into(),
        tool_name: "fake_send_email".to_string(),
        input: None,
    });
    sink.emit(AgentEvent::ToolCompleted {
        parent_call_id: None,
        call_id: "c1".into(),
        tool_name: "fake_send_email".to_string(),
        started_at_ms: None,
        input: Some(serde_json::json!({"to": "steven@example.com"})),
        output: Some(serde_json::Value::String("sent".to_string())),
        duration_ms: Some(5),
        output_bytes: Some(4),
        error: None,
        metadata: None,
    });

    let mut started_label = None;
    let mut completed = None;
    while let Ok(p) = rx.try_recv() {
        match p {
            AgentProgress::ToolCallStarted {
                display_label,
                display_detail,
                ..
            } => started_label = Some((display_label, display_detail)),
            AgentProgress::ToolCallCompleted {
                display_label,
                display_detail,
                ..
            } => completed = Some((display_label, display_detail)),
            _ => {}
        }
    }

    let (started_label, started_detail) = started_label.expect("ToolCallStarted projected");
    assert_eq!(
        started_label,
        Some("Sending email".to_string()),
        "the started label comes from the tool's own display_label, not a humanized name"
    );
    // No arguments exist yet at call-start, so the arg-derived detail is
    // absent — this is recovered on the completed event below.
    assert_eq!(started_detail, None);

    let (completed_label, completed_detail) = completed.expect("ToolCallCompleted projected");
    assert_eq!(completed_label, Some("Sending email".to_string()));
    assert_eq!(
        completed_detail,
        Some("steven@example.com".to_string()),
        "the completed detail is recomputed from the real call arguments"
    );
}

/// A running tool's progress report reaches the web channel as
/// `ToolCallProgress` for the same call; an update with no message does not.
#[tokio::test]
async fn bridge_forwards_tool_progress_with_a_message() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let bridge = OpenhumanEventBridge::new(Some(tx), "mock-model", 10);
    let sink = EventSink::new();
    sink.subscribe(bridge.clone());

    sink.emit(AgentEvent::ToolProgressDetail {
        call_id: "c1".into(),
        message: "running 4m 12s · 38.2 KB output".to_string(),
        fraction: None,
        partial: None,
    });
    sink.emit(AgentEvent::ToolProgressDetail {
        call_id: "c1".into(),
        message: String::new(),
        fraction: Some(0.5),
        partial: None,
    });

    let mut progress = Vec::new();
    while let Ok(p) = rx.try_recv() {
        if let AgentProgress::ToolCallProgress { call_id, message } = p {
            progress.push((call_id, message));
        }
    }
    assert_eq!(
        progress,
        vec![(
            "c1".to_string(),
            "running 4m 12s · 38.2 KB output".to_string()
        )]
    );
}

#[tokio::test]
async fn only_a_top_level_turn_announces_its_compaction() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    OpenhumanEventBridge::new(Some(tx), "mock-model", 10).announce_compaction();
    assert!(matches!(
        rx.try_recv(),
        Ok(AgentProgress::ContextCompacting)
    ));

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    OpenhumanEventBridge::with_scope(
        Some(tx),
        "mock-model",
        "custom",
        10,
        Some(SubagentScope {
            agent_id: "researcher".into(),
            task_id: "t1".into(),
            extended_policy: false,
        }),
        Arc::default(),
        Arc::default(),
        Arc::default(),
        Arc::default(),
        Vec::new(),
        crate::core::runtime::DispatchCtx::unbound(),
    )
    .announce_compaction();
    assert!(rx.try_recv().is_err());
}
