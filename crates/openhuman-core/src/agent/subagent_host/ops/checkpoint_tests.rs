use super::*;
use tinyagents_harness::events::{AgentEvent, EventListener, EventRecord, EventSink};
use tinyagents_harness::ids::RunId;
use tinyagents_harness::summarization::CapToolResult;
use tinyinference_llm::model::{ModelProfile, ModelResponse};

#[derive(Default)]
struct Recorder(std::sync::Mutex<Vec<AgentEvent>>);

impl EventListener for Recorder {
    fn on_event(&self, record: &EventRecord) {
        self.0.lock().unwrap().push(record.event.clone());
    }
}

struct FailingSummaryModel;

#[async_trait::async_trait]
impl ChatModel<()> for FailingSummaryModel {
    fn profile(&self) -> Option<&ModelProfile> {
        None
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        Err(tinyinference_llm::Error::Model(
            "scripted cap summary failure".into(),
        ))
    }
}

struct MessageUsageSummaryModel;

#[async_trait::async_trait]
impl ChatModel<()> for MessageUsageSummaryModel {
    fn profile(&self) -> Option<&ModelProfile> {
        None
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        let mut response = ModelResponse::assistant("checkpoint summary");
        response.message.usage = Some(tinyinference_llm::usage::Usage {
            input_tokens: 22,
            output_tokens: 5,
            total_tokens: 27,
            ..Default::default()
        });
        Ok(response)
    }
}

#[tokio::test]
async fn failed_cap_summary_is_traced_and_keeps_deterministic_fallback() {
    let run_id = RunId::new("native-child-cap");
    let events = EventSink::with_stream_id(run_id.as_str());
    let recorder = Arc::new(Recorder::default());
    events.subscribe(recorder.clone());
    let checkpoint = SubagentCheckpoint {
        chat_model: Arc::new(FailingSummaryModel),
        agent_id: "researcher".into(),
        model: "claude-opus-4-8".into(),
        max_output_tokens: 64,
        trace: Some(crate::agent::tinyagents::SupplementalTrace::new(
            events, run_id,
        )),
    };
    let results = [CapToolResult {
        call_id: "call-1",
        name: "read_file",
        output: "found a useful source",
        success: true,
    }];

    let summary = checkpoint
        .summarize_cap_hit(&results, 2)
        .await
        .expect("owner fallback preserves a successful checkpoint");
    assert!(summary.text.contains("tool-call limit (2 steps)"));
    assert!(summary
        .text
        .contains("read_file [ok]: found a useful source"));
    assert!(summary.usage.is_none());

    let records = recorder.0.lock().unwrap();
    assert!(matches!(records.as_slice(), [
        AgentEvent::ModelStarted { call_id: started, model },
        AgentEvent::ModelFailed { call_id: failed, model: failed_model, .. },
    ] if started == failed
        && started.as_str() == "native-child-cap-host-supplemental-1"
        && model == "claude-opus-4-8"
        && failed_model == model));
}

#[tokio::test]
async fn cap_summary_preserves_message_only_usage_in_product_and_native_facts() {
    let run_id = RunId::new("native-child-cap-usage");
    let events = EventSink::with_stream_id(run_id.as_str());
    let recorder = Arc::new(Recorder::default());
    events.subscribe(recorder.clone());
    let checkpoint = SubagentCheckpoint {
        chat_model: Arc::new(MessageUsageSummaryModel),
        agent_id: "researcher".into(),
        model: "claude-opus-4-8".into(),
        max_output_tokens: 64,
        trace: Some(crate::agent::tinyagents::SupplementalTrace::new(
            events, run_id,
        )),
    };
    let results = [CapToolResult {
        call_id: "call-1",
        name: "read_file",
        output: "found a useful source",
        success: true,
    }];

    let summary = checkpoint
        .summarize_cap_hit(&results, 2)
        .await
        .expect("the scripted summary succeeds");
    assert_eq!(summary.text, "checkpoint summary");
    let usage = summary
        .usage
        .expect("message-only provider usage is retained");
    assert_eq!((usage.input_tokens, usage.output_tokens), (22, 5));

    let records = recorder.0.lock().unwrap();
    let call_id = records
        .iter()
        .find_map(|event| match event {
            AgentEvent::ModelStarted { call_id, .. } => Some(call_id),
            _ => None,
        })
        .expect("native supplemental call starts");
    assert!(records.iter().any(|event| matches!(
        event,
        AgentEvent::UsageRecorded { usage }
            if usage.input_tokens == 22 && usage.output_tokens == 5
    )));
    assert!(records.iter().any(|event| matches!(
        event,
        AgentEvent::ModelCompleted {
            call_id: completed,
            usage: Some(usage),
            ..
        } if completed == call_id && usage.input_tokens == 22 && usage.output_tokens == 5
    )));
}
