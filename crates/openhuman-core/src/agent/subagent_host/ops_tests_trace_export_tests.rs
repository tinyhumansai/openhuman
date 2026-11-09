use super::*;
use crate::agent::subagent_host::{spawn_async_via_driver_tool_with_jobs, SubagentRunStatus};

/// The native journal tree must preserve the exact run identity handed to the
/// prepared child by Tiny's driver, including the host lifecycle link and the
/// durable child observations. This exercises the OpenHuman executor, not a
/// collector fixture assembled from synthetic observations.
#[tokio::test]
async fn prepared_child_run_is_exported_from_its_native_journal_lineage() {
    use tinyagents_harness::events::AgentEvent;
    use tinyagents_harness::observability::trace_export::{
        collector::SpanCollector, serialize::spans_to_ndjson, TraceContext,
    };

    let scratch = tempfile::tempdir().expect("isolated native trace fixture");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create isolated workspace");
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.clone();
    config.components.juice = false;
    config.observability.agent_tracing.capture_content = false;
    let capture_content = config.observability.agent_tracing.capture_content;
    let core = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config,
    );

    let mut response = text_response("private-child-answer-should-not-be-exported");
    let reported_usage = tinyinference_llm::usage::Usage {
        input_tokens: 120,
        output_tokens: 8,
        total_tokens: 128,
        cache_read_tokens: 20,
        charged_amount: Some(tinyinference_llm::usage::ChargedAmount::usd_micros(4_200)),
        ..Default::default()
    };
    response.usage = Some(reported_usage);
    response.message.usage = Some(reported_usage);
    let provider = ScriptedProvider::new(vec![response]);
    let mut parent_context = make_parent(provider.clone(), vec![]);
    parent_context.workspace_dir = workspace.clone();
    let mut options = with_parent(parent_context, SubagentRunOptions::default());
    options.run_context = options.run_context.with_core(Some(Arc::clone(&core)));
    options.run_context.thread_id = Some("native-trace-thread".into());
    let root_config = tinyagents_harness::context::RunConfig::new("native-trace-root")
        .with_thread("native-trace-thread");
    let root_run_id = root_config.run_id.clone();
    let parent = options.run_context.clone().into_tinyagents(root_config);
    let dispatch = parent.data.dispatch_ctx();
    let _parent_journal = crate::agent::tinyagents::journal::attach_turn_journal(
        &dispatch,
        &parent.events,
        "parent-model",
        root_run_id.clone(),
        parent.config.thread_id.clone(),
        None,
        root_run_id.clone(),
    )
    .await
    .expect("the isolated workspace attaches the root journal");
    parent.events.emit(AgentEvent::RunStarted {
        run_id: root_run_id.clone(),
        thread_id: parent.config.thread_id.clone(),
    });

    let mut options = options;
    options.checkpoint_dir = Some(scratch.path().join("checkpoints"));
    options.thread_id = Some("native-trace-thread".into());
    options.model_override = Some("claude-opus-4-8".into());
    let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
    let completion_tx = Arc::new(std::sync::Mutex::new(Some(completion_tx)));
    spawn_async_via_driver_tool_with_jobs(
        &parent,
        make_def_named_tools(&[]),
        "answer without exposing the private sentinel".into(),
        options,
        tinyagents_orchestration::subagent::SubAgentJobRegistry::new(),
        move |_target, result, effects| {
            let completion_tx = Arc::clone(&completion_tx);
            Box::pin(async move {
                if let Some(sender) = completion_tx.lock().unwrap().take() {
                    let _ = sender.send((result, effects));
                }
            })
        },
    )
    .await
    .expect("detached driver child registers");
    let (outcome, _effects) =
        tokio::time::timeout(std::time::Duration::from_secs(5), completion_rx)
            .await
            .expect("detached child completes")
            .expect("completion callback sends the host outcome");
    let outcome = outcome.expect("driver-backed child run completes");
    assert!(matches!(outcome.status, SubagentRunStatus::Completed));
    assert_eq!(provider.captured.lock().len(), 1);

    parent.events.emit(AgentEvent::RunCompleted {
        run_id: root_run_id.clone(),
        outcome: None,
    });
    parent.events.emit(AgentEvent::RunFinalized {
        run_id: root_run_id.clone(),
    });
    _parent_journal.finish_completed().await;

    let observations =
        crate::agent::tinyagents::journal::read_run_tree_events(&dispatch, root_run_id.as_str())
            .await
            .expect("lineage-indexed journals load as a tree");
    assert!(!observations.is_empty(), "the root stream is durable");
    let child_run_id = observations
        .iter()
        .find_map(|observation| match &observation.event {
            AgentEvent::SubAgentRunStarted { run_id, .. } => Some(run_id.clone()),
            _ => None,
        })
        .expect("the host lifecycle link names the actual prepared child run");
    let child_observations: Vec<_> = observations
        .iter()
        .filter(|observation| observation.run_id == child_run_id)
        .collect();
    let durable_child: Vec<_> = child_observations
        .iter()
        .map(|observation| (*observation).clone())
        .collect();
    let child_finalized_at = child_observations
        .iter()
        .find_map(|observation| match &observation.event {
            AgentEvent::RunFinalized { run_id } if run_id == &child_run_id => {
                Some(observation.ts_ms)
            }
            _ => None,
        })
        .expect("the child journal is finalized after the host adapter returns");
    let parent_ack_at = observations
        .iter()
        .find_map(|observation| match &observation.event {
            AgentEvent::SubAgentRunCompleted { run_id } if run_id == &child_run_id => {
                Some(observation.ts_ms)
            }
            _ => None,
        })
        .expect("the parent acknowledges the same child identity");
    assert!(
        parent_ack_at >= child_finalized_at,
        "parent completion follows child durable finalization"
    );
    assert!(
        child_observations.iter().all(|observation| {
            observation.parent_run_id.as_ref() == Some(&root_run_id)
                && observation.root_run_id == root_run_id
        }),
        "the persisted child stream keeps actual parent/root lineage"
    );
    let reported_call = child_observations
        .iter()
        .find_map(|observation| match &observation.event {
            AgentEvent::ModelCompleted {
                call_id,
                usage: Some(usage),
                input,
                output,
                ..
            } => {
                assert_eq!(
                    usage.charged_amount.map(|amount| amount.micros),
                    Some(4_200)
                );
                assert_eq!(usage.input_tokens, 120);
                assert_eq!(usage.output_tokens, 8);
                assert!(
                    input.is_some() && output.is_some(),
                    "the private native journal retains source observations"
                );
                Some(call_id.clone())
            }
            _ => None,
        })
        .expect("the real host model response is journaled with provider usage");
    assert!(
        child_observations.iter().any(|observation| matches!(
            &observation.event,
            AgentEvent::GenerationEnriched { call_id, enrichment }
                if call_id == &reported_call
                    && enrichment.provider_id.is_some()
                    && enrichment.pricing.is_some()
        )),
        "host provider and catalog pricing accompany the actual generation"
    );

    let trace_context = TraceContext::new("native-trace-root", None)
        .with_session_group("native-trace-thread")
        .with_capture_content(capture_content)
        .with_run_lineage(
            Some(root_run_id.as_str().into()),
            None,
            Some(root_run_id.as_str().into()),
        );
    let mut collector = SpanCollector::new(trace_context);
    for observation in &observations {
        collector.record(observation);
    }
    collector.finish(
        observations
            .iter()
            .map(|observation| observation.ts_ms)
            .max()
            .unwrap_or_default()
            .saturating_add(1),
    );
    let spans = collector.spans();
    let root_span = spans
        .iter()
        .find(|span| {
            span.kind == tinyagents_harness::observability::trace_export::SpanKind::Turn
                && span
                    .attributes
                    .get("run.id")
                    .is_some_and(|id| id == root_run_id.as_str())
        })
        .expect("the root run is represented by the trace turn span");
    let child_span = spans
        .iter()
        .find(|span| {
            span.attributes
                .get("run.id")
                .is_some_and(|id| id == child_run_id.as_str())
        })
        .expect("native collector projects the actual linked child");
    assert_eq!(
        child_span.parent_span_id.as_deref(),
        Some(root_span.span_id.as_str())
    );
    assert_eq!(root_span.attributes["run.id"], root_run_id.as_str());
    assert_eq!(root_span.attributes["root_run_id"], root_run_id.as_str());
    let generation = spans
        .iter()
        .find(|span| {
            span.attributes
                .get("gen_ai.request.model")
                .is_some_and(|_| {
                    span.attributes.get("gen_ai.usage.cost_usd") == Some(&serde_json::json!(0.0042))
                })
        })
        .expect("native projection preserves the provider-reported charge");
    assert_eq!(generation.attributes["gen_ai.usage.input_tokens"], 120);
    assert_eq!(generation.attributes["gen_ai.usage.output_tokens"], 8);
    assert!(generation
        .attributes
        .contains_key("gen_ai.pricing.input_per_mtok_usd"));
    let exported = spans_to_ndjson(
        tinyagents_harness::observability::trace_export::serialize::SpanEnvelope::Otel,
        spans,
    );
    assert!(
        !exported.contains("private-child-answer-should-not-be-exported"),
        "the native export keeps captured content off"
    );

    let child_spans = crate::core::observability::progress_tracing::build_subagent_trace(
        &durable_child,
        Some("native-trace-thread"),
        "writer",
        "child-task",
        false,
        child_finalized_at.saturating_add(1),
    );
    let child_root = child_spans
        .iter()
        .find(|span| span.kind == tinyagents_harness::observability::trace_export::SpanKind::Turn)
        .expect("the real durable child journal builds a distinct child root trace");
    assert_eq!(
        child_root.trace_id,
        format!("subagent:{}", child_run_id.as_str())
    );
    assert_eq!(child_root.attributes["thread.id"], "native-trace-thread");
    assert_eq!(child_root.attributes["agent.id"], "writer");
    assert_eq!(child_root.attributes["channel.source"], "subagent");
    assert_eq!(child_root.attributes["run.type"], "subagent");
    assert_eq!(child_root.attributes["run.id"], child_run_id.as_str());
    assert_eq!(child_root.attributes["parent_run_id"], root_run_id.as_str());
    assert_eq!(child_root.attributes["root_run_id"], root_run_id.as_str());
    let child_export = spans_to_ndjson(
        tinyagents_harness::observability::trace_export::serialize::SpanEnvelope::Langfuse,
        &child_spans,
    );
    assert!(child_export.contains("native-trace-thread"));
    assert!(child_export.contains("writer"));
    assert!(!child_export.contains("private-child-answer-should-not-be-exported"));
}
