//! Planner coverage: which of the turn's shared tools the driver registers, the
//! run policy a turn gets, and the model-call ceilings.

use super::*;
use crate::agent::tinyagents::TurnModelSource;
use crate::inference::turn_models::TurnModels;
use async_trait::async_trait;
use tinyagents_harness::tool::ToolSnapshot;
use tinyagents_harness::turn_policy::{
    DEFAULT_MODEL_CALL_TIMEOUT_SECS, LOCAL_MODEL_CALL_TIMEOUT_SECS,
};
use tinyagents_runtime::turn_driver::{SharedToolSet, TurnControls, TurnDriver};
use tinyagents_runtime::DriverRequest;
use tinyinference_llm::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
use tinytools::{Tool, ToolResult};

const GOAL_TOOLS: [&str; 3] = ["goal_get", "goal_set", "goal_complete"];

struct PlainTool;

#[async_trait]
impl Tool for PlainTool {
    fn name(&self) -> &str {
        "plain_tool"
    }

    fn description(&self) -> &str {
        "thread-independent test tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
}

struct IdleModel;

#[async_trait]
impl ChatModel<()> for IdleModel {
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
        Ok(ModelResponse::assistant("done"))
    }
}

/// Plan and prepare a turn over `tool_sets` through the real driver.
async fn prepare_with(
    model: Arc<dyn ChatModel<()>>,
    tool_sets: Vec<SharedToolSet>,
    has_thread: bool,
    tool_rules: Option<Arc<tinyagents_harness::tool::ToolRulePolicy>>,
) -> tinyagents_runtime::turn_driver::PreparedTurn<(), OpenHumanRunContext> {
    let models = TurnModelSource::from_model(model)
        .build("assembly-test-model", 0.0, None, None)
        .expect("scripted turn models build");
    let (policy, _) = turn_run_policy(
        "assembly-test-model",
        3,
        false,
        models.is_local(),
        tinyagents_harness::config::ToolDispatcher::default(),
        None,
        tool_rules,
        None,
    );
    let TurnModels {
        primary,
        routes,
        error_slot,
        ..
    } = models;
    let planner = OpenHumanTurnPlanner {
        models: turn_models_for_plan("assembly-test-model", primary, routes, &error_slot, false),
        policy,
        tool_sets,
        allowed: None,
        has_thread,
        session_deferred: Arc::new(HashSet::new()),
    };
    let request = DriverRequest {
        history: vec![tinyinference_llm::message::Message::user("hi")],
        tools: ToolSnapshot::new(Vec::new()).expect("empty snapshot"),
        run_context: tinyagents_harness::context::RunContext::new(
            tinyagents_harness::context::RunConfig::new("plan-test"),
            OpenHumanRunContext::new(),
        ),
        stream: false,
        controls: TurnControls::default(),
    };
    TurnDriver::new(Arc::new(planner), Arc::new(()))
        .prepare(request)
        .await
        .map_err(|failure| failure.error)
        .expect("turn prepares")
}

/// Prepare a turn over the real goal tools plus one ordinary tool.
async fn prepared_with(
    has_thread: bool,
    tool_rules: Option<Arc<tinyagents_harness::tool::ToolRulePolicy>>,
) -> tinyagents_runtime::turn_driver::PreparedTurn<(), OpenHumanRunContext> {
    let workspace = tempfile::TempDir::new().expect("workspace");
    let mut tools = crate::agent::goals::goal_tools(workspace.path());
    tools.push(Box::new(PlainTool));
    prepare_with(
        Arc::new(IdleModel),
        vec![Arc::new(tools)],
        has_thread,
        tool_rules,
    )
    .await
}

async fn registered_tool_names(has_thread: bool) -> Vec<String> {
    prepared_with(has_thread, None)
        .await
        .harness
        .tools()
        .names()
}

#[tokio::test]
async fn turn_tool_rules_reach_the_harness_policy() {
    let rules = tinytools::ToolRules::from_allow_deny(Vec::<String>::new(), ["plain_*"]);
    let policy = crate::tools::rules::turn_rule_policy(
        Arc::new(tinytools::ToolRuleSet::single(rules)),
        crate::tools::rules::rule_context(Some("web"), Some("orchestrator"), None),
    );
    let prepared = prepared_with(true, Some(Arc::new(policy.clone()))).await;
    assert_eq!(prepared.harness.policy().tool_rules, policy);
}

#[tokio::test]
async fn a_turn_without_rules_leaves_the_harness_policy_permissive() {
    let prepared = prepared_with(true, None).await;
    assert!(prepared.harness.policy().tool_rules.is_permissive());
}

#[tokio::test]
async fn goal_tools_are_not_registered_on_a_turn_without_a_thread() {
    let names = registered_tool_names(false).await;

    for goal_tool in GOAL_TOOLS {
        assert!(
            !names.iter().any(|name| name == goal_tool),
            "{goal_tool} must not be offered to a thread-less turn: {names:?}"
        );
    }
    assert!(
        names.iter().any(|name| name == "plain_tool"),
        "thread-independent tools still register: {names:?}"
    );
}

#[tokio::test]
async fn goal_tools_are_registered_on_a_threaded_turn() {
    let names = registered_tool_names(true).await;

    for goal_tool in GOAL_TOOLS.iter().chain(["plain_tool"].iter()) {
        assert!(
            names.iter().any(|name| name == goal_tool),
            "{goal_tool} must be offered to a threaded turn: {names:?}"
        );
    }
}

/// A model whose first (and only) byte arrives `delay` after the call starts,
/// like a local model prefilling a huge prompt (#6042).
struct SlowFirstByteModel {
    delay: std::time::Duration,
    provider: &'static str,
}

#[async_trait]
impl ChatModel<()> for SlowFirstByteModel {
    fn profile(&self) -> Option<&ModelProfile> {
        static LOCAL: std::sync::OnceLock<ModelProfile> = std::sync::OnceLock::new();
        static HOSTED: std::sync::OnceLock<ModelProfile> = std::sync::OnceLock::new();
        let (cell, provider) = if self.provider == "ollama" {
            (&LOCAL, "ollama")
        } else {
            (&HOSTED, "openrouter")
        };
        Some(cell.get_or_init(|| {
            let mut profile = ModelProfile::default();
            profile.provider = Some(provider.to_string());
            profile
        }))
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        tokio::time::sleep(self.delay).await;
        Ok(ModelResponse::assistant("done"))
    }
}

/// Prepare over a [`SlowFirstByteModel`] and run one turn on the virtual clock.
async fn run_slow_first_byte_turn(
    provider: &'static str,
    delay_secs: u64,
) -> Result<String, String> {
    let model: Arc<dyn ChatModel<()>> = Arc::new(SlowFirstByteModel {
        delay: std::time::Duration::from_secs(delay_secs),
        provider,
    });
    let run = prepare_with(model, Vec::new(), false, None)
        .await
        .run()
        .await;
    match run.partial.error {
        Some(error) => Err(error.to_string()),
        None => Ok(run.partial.run.text().unwrap_or_default()),
    }
}

/// #6042: a ~57K-token prefill on a local model can take longer than the hosted
/// 900s per-call ceiling before its first byte. A local provider must survive
/// it; a hosted provider with the identical silence must still be cut off.
#[tokio::test(start_paused = true)]
async fn local_provider_survives_a_slow_first_byte_that_a_hosted_one_does_not() {
    let delay_secs = DEFAULT_MODEL_CALL_TIMEOUT_SECS + 100;

    let hosted = run_slow_first_byte_turn("openrouter", delay_secs).await;
    let err = hosted.expect_err("hosted provider must hit the per-call ceiling");
    assert!(err.contains("timed out"), "unexpected error: {err}");

    let local = run_slow_first_byte_turn("ollama", delay_secs).await;
    assert_eq!(
        local.expect("local provider must wait out the prefill"),
        "done"
    );
}

/// The local ceiling is still a ceiling: a call that outlasts it times out.
#[tokio::test(start_paused = true)]
async fn local_provider_is_still_bounded_by_the_local_ceiling() {
    let err = run_slow_first_byte_turn("ollama", LOCAL_MODEL_CALL_TIMEOUT_SECS + 100)
        .await
        .expect_err("a call past the local ceiling must time out");
    assert!(err.contains("timed out"), "unexpected error: {err}");
}
