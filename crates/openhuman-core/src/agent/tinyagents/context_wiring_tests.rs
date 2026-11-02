use super::*;
use tinyagents_harness::turn_outcome::ToolCallOutcome;

#[test]
fn outcome_source_returns_captured_content_by_call_id() {
    let sink: ToolOutcomeSink = Default::default();
    let source = OutcomeSinkSource(sink.clone());
    assert!(source.content_for("missing").unwrap().is_none());
    sink.lock().unwrap().push(ToolCallOutcome {
        call_id: "c1".into(),
        name: "t".into(),
        arguments: serde_json::Value::Null,
        success: true,
        content: "body".into(),
        duration_ms: 0,
    });
    assert_eq!(source.content_for("c1").unwrap().as_deref(), Some("body"));
}

#[test]
fn budget_notice_thresholds_stay_at_half_and_eighty_percent() {
    assert_eq!(BUDGET_NOTICE_THRESHOLDS, [0.5, 0.8]);
}

/// Records each call it gets, then fails: the tests read the order only.
struct RecordingSummarizer(Arc<std::sync::Mutex<Vec<&'static str>>>);

#[async_trait]
impl Summarizer for RecordingSummarizer {
    async fn summarize(&self, _: &[Message]) -> tinyagents_harness::error::Result<SummaryRecord> {
        self.0.lock().unwrap().push("summarize");
        Err(tinyagents_harness::error::TinyAgentsError::Model(
            "stub".into(),
        ))
    }

    async fn merge(&self, _: &[SummaryRecord]) -> tinyagents_harness::error::Result<SummaryRecord> {
        self.0.lock().unwrap().push("merge");
        Err(tinyagents_harness::error::TinyAgentsError::Model(
            "stub".into(),
        ))
    }
}

fn recording() -> (
    Arc<std::sync::Mutex<Vec<&'static str>>>,
    Box<dyn Summarizer>,
) {
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let notice_calls = calls.clone();
    let notice: CompactionNotice = Arc::new(move || notice_calls.lock().unwrap().push("notice"));
    let summarizer = announced(Box::new(RecordingSummarizer(calls.clone())), Some(notice));
    (calls, summarizer)
}

#[tokio::test]
async fn a_summarization_is_announced_before_it_runs() {
    let (calls, summarizer) = recording();
    let _ = summarizer.summarize(&[]).await;
    let _ = summarizer
        .summarize_request(&SummaryRequest::new(Vec::new()))
        .await;
    assert_eq!(
        *calls.lock().unwrap(),
        ["notice", "summarize", "notice", "summarize"]
    );
}

#[tokio::test]
async fn merging_summaries_is_not_announced_again() {
    let (calls, summarizer) = recording();
    let _ = summarizer.merge(&[]).await;
    assert_eq!(*calls.lock().unwrap(), ["merge"]);
}

/// A compaction names the summarizer's own model in its provenance, not the
/// turn's: with `[context] summarizer_model` set the two differ.
#[tokio::test]
async fn a_compaction_records_the_summarizer_model_it_called() {
    use tinyagents_harness::context::{RunConfig, RunContext};
    use tinyagents_harness::testkit::ScriptedModel;

    let state_model = Arc::new(ScriptedModel::replies(vec![
        "{\"goal\": \"keep going\"}";
        4
    ]));
    let mw = Arc::new(compression_middleware(
        SummarizationPolicy::default().with_trigger_override(200),
        crate::config::CompactionStrategy::TaskState,
        None,
        CompactionSummarizer {
            model: state_model,
            model_id: "scaledown/summarize".into(),
            settings: Default::default(),
        },
        "turn-model",
        None,
        None,
    ));
    let mut harness: AgentHarness<()> = AgentHarness::new();
    harness.with_model_resolver(Arc::new(
        tinyagents_harness::host::FixedModelResolver::named(
            "turn-model",
            Arc::new(ScriptedModel::replies(vec!["done"])) as _,
        ),
    ));
    harness.push_middleware(mw.clone());
    harness.push_model_middleware(mw.clone());

    let mut history = vec![Message::system("system")];
    for round in 0..6 {
        history.push(Message::user(format!(
            "ask {round}: {}",
            "detail ".repeat(60)
        )));
        history.push(Message::assistant(format!(
            "answer {round}: {}",
            "result ".repeat(60)
        )));
    }
    history.push(Message::user("continue"));
    harness
        .invoke_in_context(
            &(),
            RunContext::new(RunConfig::new("compaction"), ()),
            history,
        )
        .await
        .expect("the turn runs");

    let records = mw.records();
    assert!(!records.is_empty(), "the turn compacted");
    for record in records {
        assert!(
            record.provenance.reason.contains("via scaledown/summarize"),
            "{}",
            record.provenance.reason
        );
    }
}

/// A Summary-strategy compaction with `summarizer_tool_rendering = "markers"`
/// and an output cap sends the summarizer the conversation without any tool
/// result, and caps its reply with `max_tokens`.
#[tokio::test]
async fn a_markers_compaction_sends_no_tool_output_and_caps_the_summary() {
    use tinyagents_harness::context::{RunConfig, RunContext};
    use tinyagents_harness::testkit::ScriptedModel;
    use tinyinference_llm::message::AssistantMessage;
    use tinyinference_llm::tool::ToolCall;

    let summarizer_model = Arc::new(ScriptedModel::replies(vec!["## Goal\nkeep going"; 4]));
    let mw = Arc::new(compression_middleware(
        SummarizationPolicy::default().with_trigger_override(200),
        crate::config::CompactionStrategy::Summary,
        None,
        CompactionSummarizer {
            model: summarizer_model.clone(),
            model_id: "scaledown/summarize".into(),
            settings: crate::config::SummarizerSettings {
                tool_rendering: tinyagents_harness::summarization::ToolRendering::Markers,
                max_output_tokens: Some(4_000),
            },
        },
        "turn-model",
        None,
        None,
    ));
    let mut harness: AgentHarness<()> = AgentHarness::new();
    harness.with_model_resolver(Arc::new(
        tinyagents_harness::host::FixedModelResolver::named(
            "turn-model",
            Arc::new(ScriptedModel::replies(vec!["done"])) as _,
        ),
    ));
    harness.push_middleware(mw.clone());
    harness.push_model_middleware(mw.clone());

    let mut history = vec![Message::system("system")];
    for round in 0..12 {
        history.push(Message::user(format!(
            "ask {round}: {}",
            "detail ".repeat(40)
        )));
        history.push(Message::Assistant(AssistantMessage {
            id: None,
            content: Vec::new(),
            tool_calls: vec![ToolCall::new(
                format!("call-{round}"),
                "lookup",
                serde_json::json!({"query": format!("q{round}")}),
            )],
            usage: None,
            origin: None,
        }));
        history.push(Message::tool(
            format!("call-{round}"),
            format!("TOOL-OUTPUT-{round} {}", "row ".repeat(40)),
        ));
        history.push(Message::assistant(format!("answer {round}")));
    }
    history.push(Message::user("continue"));
    harness
        .invoke_in_context(
            &(),
            RunContext::new(RunConfig::new("compaction"), ()),
            history,
        )
        .await
        .expect("the turn runs");

    let requests = summarizer_model.requests();
    assert!(!requests.is_empty(), "the turn compacted");
    for request in requests {
        let sent: String = request.messages.iter().map(Message::text).collect();
        assert!(!sent.contains("TOOL-OUTPUT-"), "{sent}");
        assert!(sent.contains("[used lookup: "), "{sent}");
        assert_eq!(request.max_tokens, Some(4_000));
    }
}

fn idle_history() -> Vec<Message> {
    let mut history = vec![Message::system("system prompt")];
    for round in 0..8 {
        history.push(Message::user(format!(
            "ask {round}: {}",
            "detail ".repeat(60)
        )));
        history.push(Message::assistant(format!(
            "answer {round}: {}",
            "result ".repeat(60)
        )));
    }
    history
}

/// Outside a turn, the compression middleware folds the head into one
/// user-role summary right after the system prompt and keeps the tail.
#[tokio::test]
async fn an_idle_fold_puts_the_summary_after_the_system_prompt() {
    use tinyagents_harness::testkit::ScriptedModel;

    let summarizer_model = Arc::new(ScriptedModel::replies(vec!["## Goal\nIDLE-SUMMARY"; 4]));
    let history = idle_history();
    let compacted = compact_history(
        &history,
        IdleFold {
            min_tokens: 100,
            keep_recent_tokens: Some(150),
            strategy: crate::config::CompactionStrategy::Summary,
            context_window: None,
            model: "turn-model",
        },
        CompactionSummarizer {
            model: summarizer_model.clone(),
            model_id: "scaledown/summarize".into(),
            settings: Default::default(),
        },
    )
    .await
    .expect("the fold runs")
    .expect("the history is folded");

    assert_eq!(compacted[0], history[0], "the system prompt is kept as is");
    assert!(matches!(compacted[1], Message::User(_)));
    assert!(compacted[1].text().contains("IDLE-SUMMARY"));
    assert_eq!(
        compacted.last(),
        history.last(),
        "the tail is kept verbatim"
    );
    assert!(compacted.len() < history.len());
    assert_eq!(summarizer_model.requests().len(), 1);
}

/// A conversation of one message leaves nothing older than the tail to fold.
#[tokio::test]
async fn an_idle_fold_with_nothing_old_enough_returns_none() {
    use tinyagents_harness::testkit::ScriptedModel;

    let summarizer_model = Arc::new(ScriptedModel::replies(vec!["unused"]));
    let folded = compact_history(
        &[
            Message::system("system prompt"),
            Message::user(format!("only ask: {}", "detail ".repeat(200))),
        ],
        IdleFold {
            min_tokens: 100,
            keep_recent_tokens: Some(1_000_000),
            strategy: crate::config::CompactionStrategy::Summary,
            context_window: None,
            model: "turn-model",
        },
        CompactionSummarizer {
            model: summarizer_model.clone(),
            model_id: "m".into(),
            settings: Default::default(),
        },
    )
    .await
    .expect("the fold runs");
    assert!(folded.is_none());
    assert!(summarizer_model.requests().is_empty());
}
