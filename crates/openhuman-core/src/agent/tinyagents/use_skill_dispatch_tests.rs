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
use crate::agent::harness::host_definition::AgentDefinitionRegistry;
use crate::agent::harness::ParentExecutionContext;
use crate::agent::tinyagents::tool_registration::shared_tool_adapter;
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
use tinytools_agent::dialect::ToolCallFormat;

/// A stub archetype-delegate tool: only its name has to match `image_agent`'s
/// `delegate_name` ("create_image", set in
/// `agent/registry/agents/image_agent/agent.toml`) for
/// `DelegationDispatch::for_tool` to select the archetype path. The typed
/// archetype test dispatches straight to
/// `execute_archetype_delegation_with_live_parent`; the separate packed-target
/// binding regression intentionally exercises this stub's own authorization
/// and execution methods.
struct StubCreateImage(&'static str);

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
        Ok(ToolResult::success(self.0))
    }

    async fn prepare_authorization(
        &self,
        _args: &serde_json::Value,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(Some(json!({"target": self.0})))
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
        host_env: Default::default(),
    }
}

/// Builds the `use_skill` registration exactly as
/// `host_tool_set_builder` does: a durable registry containing the
/// real `UseSkillTool` plus one packed archetype-delegate stub, a
/// `PackRegistryHandle` bound to it, and the `CanonicalSharedToolAdapter`
/// wrapping `use_skill` that the harness would have registered.
fn build_use_skill_dispatch_with_handle() -> (UseSkillDispatch, PackRegistryHandle) {
    let handle = PackRegistryHandle::default();
    let use_skill_tool: Box<dyn Tool> = Box::new(UseSkillTool::new(
        handle.clone(),
        crate::tools::toolpacks::CATALOG,
    ));
    let create_image_tool: Box<dyn Tool> = Box::new(StubCreateImage("original"));
    let durable: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![use_skill_tool, create_image_tool]);
    handle.bind(Arc::downgrade(&durable));

    let adapter = shared_tool_adapter(vec![durable], tinyagents_harness::tool::packs::USE_SKILL)
        .expect("use_skill resolves in the durable registry it was just placed in");
    (
        UseSkillDispatch::new(Arc::new(adapter), handle.clone()),
        handle,
    )
}

fn build_use_skill_dispatch() -> UseSkillDispatch {
    build_use_skill_dispatch_with_handle().0
}

#[tokio::test]
async fn use_skill_dispatch_keeps_authorized_target_after_registry_rebind() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let (dispatch, handle) = build_use_skill_dispatch_with_handle();
    let arguments = json!({
        "skill": "media",
        "tool": "create_image",
        "args": { "prompt": "a red bicycle", "blocking": true },
    });
    let pinned = dispatch
        .bind_call(&arguments)
        .expect("bind succeeds")
        .expect("known packed target binds");
    assert_eq!(
        pinned.output_origin(),
        tinyagents_harness::host::ContentOrigin::Agent,
        "typed packed dispatch must keep the delegated-agent output policy"
    );

    let replacement: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![
        Box::new(UseSkillTool::new(
            handle.clone(),
            crate::tools::toolpacks::CATALOG,
        )),
        Box::new(StubCreateImage("replacement")),
    ]);
    handle.bind(Arc::downgrade(&replacement));

    let facts = pinned
        .tool()
        .prepare_authorization(&arguments)
        .await
        .expect("authorization facts are returned");
    let result = pinned
        .tool()
        .execute(arguments)
        .await
        .expect("bound call returns a result");

    assert_eq!(facts, Some(json!({"target": "original"})));
    assert_eq!(result.text(), "original");
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
