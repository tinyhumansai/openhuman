use super::*;
use async_trait::async_trait;
use tinyagents_harness::testkit::ScriptedModel;
use tinyinference_llm::message::AssistantMessage;
use tinyinference_llm::model::{ChatModel, ModelProfile, ModelResponse};
use tinyinference_llm::tool::ToolCall;
use tinytools::ToolResult;

struct PingTool;
#[async_trait]
impl Tool for PingTool {
    fn name(&self) -> &str {
        "ping"
    }
    fn description(&self) -> &str {
        "ping"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(&self, _a: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("pong"))
    }
}

#[tokio::test]
async fn channel_turn_runs_through_the_graph() {
    let registry: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(PingTool)]);
    let mut history = vec![TranscriptMessage::user("ping please")];
    let scripted: Arc<dyn ChatModel<()>> = Arc::new(ScriptedModel::new(vec![
        ModelResponse {
            message: AssistantMessage {
                id: None,
                content: Vec::new(),
                tool_calls: vec![ToolCall::new("p", "ping", serde_json::json!({}))],
                usage: None,
                origin: None,
            },
            usage: None,
            finish_reason: Some("tool_calls".to_string()),
            raw: None,
            resolved_model: None,
            continue_turn: None,
            served_from_cache: false,
            correlation: None,
            resolved_route: None,
        },
        ModelResponse::assistant("channel done"),
    ]));
    let mut profile = ModelProfile::default();
    profile.tool_calling = true;
    profile.parallel_tool_calls = true;
    let text = run_channel_turn_via_graph(
        TurnModelSource::from_model_with_profile(scripted, profile),
        &mut history,
        registry,
        vec![],
        None,
        "mock-model",
        0.0,
        10,
        MultimodalConfig::default(),
        MultimodalFileConfig::default(),
        None,
        None,
        tinyagents_definition::SandboxMode::None,
        Default::default(),
        Default::default(),
    )
    .await
    .expect("channel graph turn runs");
    assert_eq!(text.text, "channel done");
    assert!(history.iter().any(|m| m.content.contains("pong")));
}

/// Regression: the channel path must PAUSE on `ask_user_clarification`, not feed
/// the tool's output back to the model.
///
/// The scripted model offers a second response ("built it without asking"). A
/// run that consumes it is the bug this fixes: `early_exit_tools` was `&[]`, so
/// the question came back as an ordinary successful tool result and the model
/// answered its own question. With the pause wired up the second response is
/// never reached and the turn ends on the question itself.
#[tokio::test]
async fn channel_turn_pauses_on_ask_user_clarification() {
    let registry: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(
        tinyagents_harness::tools::AskClarificationTool::new(),
    )]);
    let mut history = vec![TranscriptMessage::user("build me a workflow")];
    let scripted: Arc<dyn ChatModel<()>> = Arc::new(ScriptedModel::new(vec![
        ModelResponse {
            message: AssistantMessage {
                id: None,
                content: Vec::new(),
                tool_calls: vec![ToolCall::new(
                    "c",
                    "ask_user_clarification",
                    serde_json::json!({ "question": "Which three sources?" }),
                )],
                usage: None,
                origin: None,
            },
            usage: None,
            finish_reason: Some("tool_calls".to_string()),
            raw: None,
            resolved_model: None,
            continue_turn: None,
            served_from_cache: false,
            correlation: None,
            resolved_route: None,
        },
        ModelResponse::assistant("built it without asking"),
    ]));
    let mut profile = ModelProfile::default();
    profile.tool_calling = true;
    profile.parallel_tool_calls = true;
    let text = run_channel_turn_via_graph(
        TurnModelSource::from_model_with_profile(scripted, profile),
        &mut history,
        registry,
        vec![],
        None,
        "mock-model",
        0.0,
        10,
        MultimodalConfig::default(),
        MultimodalFileConfig::default(),
        None,
        None,
        tinyagents_definition::SandboxMode::None,
        Default::default(),
        Default::default(),
    )
    .await
    .expect("channel graph turn runs");

    assert_eq!(
        text.text, "Which three sources?",
        "the turn must end on the question; got the model's own follow-up, so the pause did not fire"
    );
    let last = history.last().expect("history is not empty");
    assert_eq!(last.role, "assistant");
    assert_eq!(
        last.content, "Which three sources?",
        "the question stands in for the final assistant turn the paused run never produced"
    );
}

#[test]
fn channel_current_images_are_available_to_vision_delegation() {
    use tinyagents_session::transcript::TranscriptPart;
    let mut current = TranscriptMessage::user("inspect the attached image");
    current.parts = Some(vec![
        TranscriptPart::Text {
            text: current.content.clone(),
        },
        TranscriptPart::Image {
            url: "uploads/thread/image.png".into(),
        },
    ]);
    let history = vec![
        TranscriptMessage::user("[IMAGE:uploads/old.png]"),
        current,
        TranscriptMessage::assistant("delegating"),
    ];
    let mut context = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    seed_channel_attachments(&mut context, &history);
    assert_eq!(
        context.attachment_placeholders.as_ref(),
        &vec!["[IMAGE:uploads/thread/image.png]".to_string()]
    );
    assert!(crate::agent::attachments::should_forward_parent_images(
        "describe the image"
    ));
    assert!(!crate::agent::attachments::should_forward_parent_images(
        "[IMAGE:uploads/explicit.png]"
    ));
}
