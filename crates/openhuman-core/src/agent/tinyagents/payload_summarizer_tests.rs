use super::*;
use crate::agent::harness::definition::{
    DefinitionSource, ModelSpec, PromptSource, SandboxMode, ToolScope,
};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse, ModelStream};

fn dummy_definition() -> AgentDefinition {
    AgentDefinition {
        id: "summarizer".into(),
        when_to_use: "test".into(),
        display_name: Some("Summarizer".into()),
        system_prompt: PromptSource::Inline("test prompt".into()),
        omit_identity: true,
        omit_memory_context: true,
        omit_safety_preamble: true,
        model: ModelSpec::Hint("summarization".into()),
        temperature: 0.2,
        tools: ToolScope::Named(vec![]),
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        deferred_tools: Vec::new(),
        max_iterations: 1,
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        tokenjuice_compression: crate::inference::tokenjuice::AgentTokenjuiceCompression::Auto,
        subagents: vec![],
        delegate_name: None,
        agent_tier: crate::agent::harness::definition::AgentTier::Worker,
        source: DefinitionSource::Builtin,
        graph: Default::default(),
    }
}

#[test]
fn unary_summarizer_child_inherits_cancellation_workspace_and_lineage() {
    let cancellation = tinyagents_harness::cancel::CancellationToken::new();
    let workspace = tinytools::WorkspaceDescriptor::new("/work/action");
    let parent = OpenHumanRunContext::new()
        .with_cancellation(cancellation.clone())
        .with_workspace(workspace.clone())
        .into_tinyagents(RunConfig::new("parent").with_thread("thread-a"));

    let child = unary_child_context(&parent, "summarizer", 1, 128).expect("child context");

    assert_eq!(child.workspace, Some(workspace));
    assert_eq!(child.thread_id().map(|id| id.as_str()), Some("thread-a"));
    assert_eq!(child.depth(), 1);
    assert_eq!(child.data.spawn_depth, 1);
    cancellation.cancel();
    assert!(child.cancellation.is_cancelled());
}

struct UnaryOnlyModel(AtomicBool);

#[async_trait]
impl ChatModel<()> for UnaryOnlyModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        self.0.store(true, Ordering::SeqCst);
        Ok(ModelResponse::assistant("condensed summary"))
    }

    async fn stream(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelStream> {
        panic!("a payload summary must not stream into the user-visible parent turn")
    }
}

#[tokio::test]
async fn unary_summarizer_child_keeps_summary_output_off_the_streaming_path() {
    let parent = OpenHumanRunContext::new().into_tinyagents(RunConfig::new("parent"));
    let child = unary_child_context(&parent, "summarizer", 1, 128).expect("child context");
    let model = Arc::new(UnaryOnlyModel(AtomicBool::new(false)));
    let mut harness: AgentHarness<(), OpenHumanRunContext> = AgentHarness::new();
    harness
        .register_model("summary", model.clone())
        .set_default_model("summary");

    let run = harness
        .invoke_in_context(&(), child, vec![Message::user("summarize this")])
        .await
        .expect("unary summary run");

    assert!(model.0.load(Ordering::SeqCst));
    assert_eq!(run.text().as_deref(), Some("condensed summary"));
}

#[test]
fn preparing_without_a_parent_turn_is_an_error() {
    let parent = OpenHumanRunContext::new().into_tinyagents(RunConfig::new("parent"));
    let summarizer = SubagentPayloadSummarizer::new(dummy_definition());
    let error = summarizer
        .prepare(&parent)
        .err()
        .expect("no ParentExecutionContext");
    assert!(error.to_string().contains("ParentExecutionContext"));
}
