use super::*;

use std::sync::Arc;

use tinyagents_definition::DefinitionRegistry;
use tinyagents_harness::host::{
    BudgetGate, ContextComposer, LearningSink, ModelResolver, ProgressSink, SecurityGate,
    ToolOutcomeClassifier,
};

fn inputs() -> (OpenHumanHostBundleInputs, OpenHumanRunContext) {
    let (progress, _receiver) = tokio::sync::mpsc::channel(4);
    let mut turn = OpenHumanRunContext::new();
    turn.thread_id = Some("thread-for-bundle".to_string());
    turn.progress = Some(progress);
    (
        OpenHumanHostBundleInputs {
            config: Arc::new(Config::default()),
            definitions: Arc::new(AgentDefinitionRegistry::builtins_only()),
            security_policy: Arc::new(SecurityPolicy::default()),
            tool_sets: Vec::new(),
            tool_policy: None,
            post_turn_hooks: Vec::new(),
            session_definition: None,
        },
        turn,
    )
}

#[test]
fn factory_records_every_concrete_adapter_in_one_bundle() {
    let (inputs, turn) = inputs();
    let host = OpenHumanHostBundleFactory::build(inputs, &turn);

    let context: Arc<dyn ContextComposer> = host.context.clone();
    let definitions: Arc<dyn DefinitionRegistry> = host.definitions.clone();
    let security: Arc<dyn SecurityGate> = host.security.clone();
    let models: Arc<dyn ModelResolver<()>> = host.models.clone();
    let budget: Arc<dyn BudgetGate> = host.budget.clone();
    let progress: Arc<dyn ProgressSink> = host.progress.clone();
    let learning: Arc<dyn LearningSink> = host.learning.clone();
    let outcomes: Arc<dyn ToolOutcomeClassifier> = host.tool_outcomes.clone();

    assert!(Arc::ptr_eq(&context, &host.capabilities.context));
    assert!(Arc::ptr_eq(&definitions, &host.capabilities.definitions));
    assert!(Arc::ptr_eq(&security, &host.capabilities.security));
    assert!(Arc::ptr_eq(&models, &host.capabilities.models));
    assert!(host.capabilities.memory.is_none());
    assert!(host.capabilities.experience.is_none());
    assert!(Arc::ptr_eq(
        &budget,
        host.capabilities.budget.as_ref().expect("budget")
    ));
    assert!(Arc::ptr_eq(
        &progress,
        host.capabilities.progress.as_ref().expect("progress")
    ));
    assert!(Arc::ptr_eq(
        &learning,
        host.capabilities.learning.as_ref().expect("learning")
    ));
    assert!(Arc::ptr_eq(
        &outcomes,
        host.capabilities
            .tool_outcomes
            .as_ref()
            .expect("tool outcomes"),
    ));
}

struct DeferredBrowser;

#[async_trait::async_trait]
impl tinytools::Tool for DeferredBrowser {
    fn name(&self) -> &str {
        "browser_open"
    }
    fn description(&self) -> &str {
        "Open a website"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    fn exposure(&self) -> tinytools::ToolExposure {
        tinytools::ToolExposure::Deferred
    }
    async fn execute(&self, _: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
        Ok(tinytools::ToolResult::success("ok"))
    }
}

#[tokio::test]
async fn hosted_orchestrator_allows_deferred_browser_discovery() {
    let (mut inputs, turn) = inputs();
    inputs.tool_sets = vec![Arc::new(vec![Box::new(DeferredBrowser)])];
    let host = OpenHumanHostBundleFactory::build(inputs, &turn);
    let definition = host
        .definitions
        .resolve("orchestrator")
        .await
        .expect("resolve")
        .expect("orchestrator");
    assert!(definition.tools.contains(&"browser_open".to_string()));
}
