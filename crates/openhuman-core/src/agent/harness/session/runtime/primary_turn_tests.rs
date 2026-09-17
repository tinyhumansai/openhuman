use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tinyinference::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};

use super::*;

struct NeverCalledTool;

#[async_trait]
impl crate::tools::Tool for NeverCalledTool {
    fn name(&self) -> &str {
        "never_called"
    }

    fn description(&self) -> &str {
        "Test-only tool that must not be exposed by the mode gate."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<crate::tools::ToolResult> {
        panic!("mode-gate test tool must never execute")
    }
}

#[derive(Default)]
struct RecordingModel {
    requests: Mutex<Vec<ModelRequest>>,
}

#[async_trait]
impl ChatModel<()> for RecordingModel {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::LazyLock<ModelProfile> =
            std::sync::LazyLock::new(ModelProfile::default);
        Some(&PROFILE)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        self.requests.lock().push(request);
        Ok(ModelResponse::assistant("mock answer"))
    }

    async fn stream(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelStream> {
        let response = self.invoke(state, request).await?;
        Ok(Box::pin(futures::stream::iter(vec![
            ModelStreamItem::Started,
            ModelStreamItem::Completed(response),
        ])))
    }
}

fn agent(model: Arc<RecordingModel>, workspace: &std::path::Path) -> Agent {
    Agent::builder()
        .chat_model(model)
        .tools(vec![Box::new(NeverCalledTool)])
        .memory(crate::memory::test_support::noop_memory())
        .tool_dispatcher(Box::new(crate::agent::dispatcher::NativeToolDispatcher))
        .workspace_dir(workspace.to_path_buf())
        .model_name("qwen38-openhuman".to_string())
        .build()
        .expect("build test agent")
}

#[tokio::test]
async fn greeting_and_general_question_each_make_one_call_with_zero_tools() {
    for prompt in ["hey", "explain why the sky is blue"] {
        let workspace = tempfile::tempdir().expect("temp workspace");
        let model = Arc::new(RecordingModel::default());
        let mut agent = agent(model.clone(), workspace.path());

        let answer = agent
            .run_primary_interactive(
                prompt,
                PrimaryTurnMode::Chat,
                OrchestrationEngine::Goose,
                "chat-test",
                None,
            )
            .await
            .expect("direct chat succeeds");
        assert_eq!(answer, "mock answer");
        let requests = model.requests.lock();
        assert_eq!(requests.len(), 1, "{prompt}");
        assert!(requests[0].tools.is_empty(), "{prompt}");
        assert_eq!(requests[0].tool_choice, ToolChoice::None, "{prompt}");
        assert!(requests[0]
            .messages
            .iter()
            .any(|message| message.text().contains("## Chat mode")));
        assert!(!requests[0]
            .messages
            .iter()
            .any(|message| message.text().contains("active_goal")));
    }
}

#[tokio::test]
async fn assist_and_agent_dispatch_through_goose_with_no_gate4_tools() {
    for mode in [PrimaryTurnMode::Assist, PrimaryTurnMode::Agent] {
        let workspace = tempfile::tempdir().expect("temp workspace");
        let model = Arc::new(RecordingModel::default());
        let mut agent = agent(model.clone(), workspace.path());

        let answer = agent
            .run_primary_interactive(
                "perform the requested action",
                mode,
                OrchestrationEngine::Goose,
                mode.as_str(),
                None,
            )
            .await
            .expect("Goose turn succeeds");
        assert_eq!(answer, "mock answer");
        let requests = model.requests.lock();
        assert_eq!(requests.len(), 1, "mode={}", mode.as_str());
        assert!(requests[0].tools.is_empty());
        assert!(requests[0]
            .messages
            .iter()
            .any(|message| message.text().contains("Complete the user's request")));
    }
}

#[test]
fn tinyagents_setting_keeps_the_rollback_path() {
    std::thread::Builder::new()
        .name("phase7-tinyagents-rollback".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(async {
                    let workspace = tempfile::tempdir().expect("temp workspace");
                    let model = Arc::new(RecordingModel::default());
                    let mut agent = agent(model.clone(), workspace.path());

                    let answer = agent
                        .run_primary_interactive(
                            "hey",
                            PrimaryTurnMode::Chat,
                            OrchestrationEngine::Tinyagents,
                            "rollback",
                            None,
                        )
                        .await
                        .expect("TinyAgents rollback succeeds");
                    assert_eq!(answer, "mock answer");
                    assert_eq!(model.requests.lock().len(), 1);
                });
        })
        .expect("spawn rollback test")
        .join()
        .expect("rollback test thread");
}
