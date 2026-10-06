//! Regression coverage for R3: `use_skill` must reach a packed archetype
//! delegation (`create_image`, `do_crypto`, `make_presentation`, …) through
//! the SAME live-parent typed dispatch a natively advertised delegate tool
//! gets, not the plain `Tool::execute_with_context` path that has no parent
//! to recurse into.
//!
//! Before this fix, [`UseSkillDispatch`] did not exist and `use_skill` was
//! registered with `harness.register_tool(adapter)` — a plain registration
//! that always calls `Tool::execute_with_context(.., None)`. Reaching
//! `create_image` (the `image_agent` archetype delegate) through `use_skill`
//! therefore always failed with "delegation requires a live harness run
//! context." even when the model's actual turn had one. Reverting
//! `use_skill`'s registration to `harness.register_tool(adapter)` reproduces
//! that failure and is the fastest way to see these tests fail red.

use super::*;
use crate::agent::harness::definition::AgentDefinitionRegistry;
use crate::agent::harness::ParentExecutionContext;
use crate::agent::prompts::ToolCallFormat;
use crate::agent::tinyagents::tools::CanonicalSharedToolAdapter;
use async_trait::async_trait;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use tinyagents_harness::context::RunConfig;
use tinyagents_harness::tool::packs::{PackRegistryHandle, UseSkillTool};
use tinyagents_harness::CallId;
use tinyinference_llm::message::Message;
use tinyinference_llm::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
use tinytools::{Tool, ToolCallOptions, ToolResult};

/// A stub archetype-delegate tool: only its name has to match `image_agent`'s
/// `delegate_name` ("create_image", set in
/// `agent/registry/agents/image_agent/agent.toml`) for
/// `DelegationDispatch::for_tool` to select the archetype path. Real
/// archetype dispatch never calls the wrapped tool's own `execute` — it
/// dispatches straight to `execute_archetype_delegation_with_live_parent` —
/// so this stub's body is unreachable in a passing run.
struct StubCreateImage;

#[async_trait]
impl Tool for StubCreateImage {
    fn name(&self) -> &str {
        "create_image"
    }
    fn description(&self) -> &str {
        "stub archetype delegate for image_agent"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({})
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        unreachable!("archetype dispatch must not fall back to the wrapped tool's own execute")
    }
}

/// A `ChatModel` that answers any request, so a real (short) sub-agent turn
/// can run to completion without a network dependency or a canary match.
struct AnyAnswerModel;

#[async_trait]
impl ChatModel<()> for AnyAnswerModel {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::OnceLock<ModelProfile> = std::sync::OnceLock::new();
        Some(PROFILE.get_or_init(|| {
            let mut profile = ModelProfile::default();
            profile.tool_calling = true;
            profile
        }))
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        Ok(ModelResponse::assistant("a generated image description"))
    }
}

#[allow(dead_code)]
fn unused_message_ref(_m: &Message) {}

fn parent_execution_context(workspace_dir: &Path) -> ParentExecutionContext {
    ParentExecutionContext {
        runtime_config: None,
        workspace_descriptor: None,
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: ["image_agent".to_string()].into_iter().collect(),
        turn_model_source: crate::agent::tinyagents::TurnModelSource::from_model(Arc::new(
            AnyAnswerModel,
        )),
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.2,
        workspace_dir: workspace_dir.to_path_buf(),
        agent_config: Default::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "use-skill-dispatch-tests".into(),
        channel: "test".into(),
        connected_integrations: Vec::new(),
        tool_call_format: ToolCallFormat::Native,
        session_key: "use-skill-dispatch-tests".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    }
}

/// Builds the `use_skill` registration exactly as
/// `register_turn_tools_and_agents` does: a durable registry containing the
/// real `UseSkillTool` plus one packed archetype-delegate stub, a
/// `PackRegistryHandle` bound to it, and the `CanonicalSharedToolAdapter`
/// wrapping `use_skill` that the harness would have registered.
fn build_use_skill_dispatch() -> UseSkillDispatch {
    let handle = PackRegistryHandle::default();
    let use_skill_tool: Box<dyn Tool> = Box::new(UseSkillTool::new(
        handle.clone(),
        crate::tools::toolpacks::CATALOG,
    ));
    let create_image_tool: Box<dyn Tool> = Box::new(StubCreateImage);
    let durable: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![use_skill_tool, create_image_tool]);
    handle.bind(Arc::downgrade(&durable));

    let adapter = CanonicalSharedToolAdapter::for_name(
        vec![durable],
        tinyagents_harness::tool::packs::USE_SKILL,
    )
    .expect("use_skill resolves in the durable registry it was just placed in");
    UseSkillDispatch::new(Arc::new(adapter), handle)
}

/// The regression itself: `use_skill { skill: "media", tool: "create_image",
/// args: { prompt: "x", blocking: true } }` must reach the live-parent
/// archetype-delegation path instead of failing with "delegation requires a
/// live harness run context."
#[tokio::test]
async fn use_skill_dispatch_reaches_live_parent_for_packed_archetype_delegate() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let dispatch = build_use_skill_dispatch();
    let workspace = tempfile::TempDir::new().expect("workspace");

    let parent_data = crate::agent::tinyagents::host::OpenHumanRunContext::new()
        .with_parent(parent_execution_context(workspace.path()));
    let parent = parent_data.into_tinyagents(RunConfig::new("use-skill-dispatch-parent"));

    let result = dispatch
        .execute(
            &(),
            CallId::new("use-skill-call"),
            json!({
                "skill": "media",
                "tool": "create_image",
                "args": { "prompt": "a red bicycle", "blocking": true },
            }),
            ToolCallOptions::default(),
            &parent,
        )
        .await
        .expect("dispatch returns a tool result rather than an Err");

    let output = result.output();
    assert!(
        !output.contains("requires a live harness run context"),
        "use_skill must hand the packed delegation its live parent, not fail on the \
         standalone-caller guard: {output}"
    );
}

/// The disclosure half (no `tool` named) needs no live parent at all, and
/// must keep behaving exactly like `UseSkillTool::execute_with_context` —
/// [`UseSkillDispatch`] delegates to it rather than re-deriving the listing.
#[tokio::test]
async fn use_skill_dispatch_disclosure_half_delegates_to_use_skill_tool() {
    let dispatch = build_use_skill_dispatch();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let parent_data = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    let parent = parent_data.into_tinyagents(RunConfig::new("use-skill-disclosure-parent"));
    let _ = workspace; // keep the temp dir alive for symmetry with the other test

    let result = dispatch
        .execute(
            &(),
            CallId::new("use-skill-disclosure-call"),
            json!({ "skill": "media" }),
            ToolCallOptions::default(),
            &parent,
        )
        .await
        .expect("disclosure half returns a tool result");

    assert!(!result.is_error, "{}", result.output());
    assert!(
        result.output().contains("create_image"),
        "the rendered pack listing must include the bound tool: {}",
        result.output()
    );
}

/// A `tool` the pack does not own (or that is not bound) is the not-found
/// path, and also needs no live parent.
#[tokio::test]
async fn use_skill_dispatch_unknown_tool_reports_not_found() {
    let dispatch = build_use_skill_dispatch();
    let parent_data = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    let parent = parent_data.into_tinyagents(RunConfig::new("use-skill-not-found-parent"));

    let result = dispatch
        .execute(
            &(),
            CallId::new("use-skill-not-found-call"),
            json!({ "skill": "media", "tool": "not_a_real_tool" }),
            ToolCallOptions::default(),
            &parent,
        )
        .await
        .expect("not-found half returns a tool result");

    assert!(result.is_error);
    assert!(
        result.output().contains("not_a_real_tool"),
        "{}",
        result.output()
    );
}
