//! A turn whose model calls a host tool, under an autonomy policy that asks
//! TinySecurity to classify the call while the module cannot be reached
//! (`[modules] enabled = false`). With security enforced the call is refused
//! (`[policy-blocked]`); with security disabled, by the host
//! (`RuntimeBuilder::security`) or by a build without the `security-module`
//! feature, the tool runs.

#![allow(dead_code)] // Each test target includes the shared scenario but runs one case.

use crate::common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::{
    chat_completion, chat_requests, offline_config, route, runtime, scripted_provider,
    stub_backend, tool_call_completion, tool_results,
};
use openhuman_embed::config::{PromptScreenConfig, SecurityModeConfig};
use openhuman_embed::{
    AgentDefinitionSpec, AgentSpec, HostTurnTools, Runtime, Tool, ToolScopeSpec, Workspace,
};
use serde_json::{json, Value};

/// A writing host tool that counts its runs.
struct Probe(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl Tool for Probe {
    fn name(&self) -> &str {
        "probe"
    }
    fn description(&self) -> &str {
        "Perform a host operation"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn permission_level(&self) -> tinytools::PermissionLevel {
        tinytools::PermissionLevel::Write
    }
    async fn execute(&self, _: Value) -> anyhow::Result<tinytools::ToolResult> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(tinytools::ToolResult::success("probe ran"))
    }
}

/// What the turn did with the model's `probe` call.
pub struct Outcome {
    /// How many times the tool ran.
    pub runs: usize,
    /// The tool result the model saw.
    pub tool_result: String,
    /// The runtime's report of whether security is enforced.
    pub security_enforced: bool,
}

/// Run one turn; `security` is the host's `[security]` knob, if it sets one.
/// The prompt screen is turned off so the turn reaches the model in every
/// build and only the tool gate is under test.
pub fn tool_turn(security: Option<SecurityModeConfig>) -> Outcome {
    runtime().block_on(async move {
        tokio::spawn(async move {
            let backend = stub_backend().await;
            let model = scripted_provider(
                vec![tool_call_completion("probe", "{}"), chat_completion("done")],
                "done",
            )
            .await;
            let mut config = offline_config();
            config.autonomy.enabled = true;
            config.modules.enabled = false;
            let mut builder = Runtime::builder()
                .config(config)
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .prompt_screen(PromptScreenConfig::off());
            if let Some(security) = security {
                builder = builder.security(security);
            }
            let runtime = builder.build().await.expect("runtime");
            let security_enforced = runtime.capabilities().configuration.security_enforced;
            let runs = Arc::new(AtomicUsize::new(0));
            let tool_runs = runs.clone();
            let agent = runtime
                .agent(
                    AgentSpec::new("security")
                        .provider(route(&model, "fixture"))
                        .definition(
                            AgentDefinitionSpec::new()
                                .bare_prompt("Use probe then answer")
                                .tools(ToolScopeSpec::HostOnly),
                        )
                        .tools(move |_| {
                            let runs = tool_runs.clone();
                            Box::pin(async move {
                                HostTurnTools::advertised(vec![Box::new(Probe(runs))])
                            })
                        }),
                )
                .expect("agent");

            agent.run("use the probe").await.expect("turn");

            let requests = chat_requests(&model).await;
            assert!(requests.len() >= 2, "the model saw the tool result");
            let tool_result = tool_results(&requests[1]);
            runtime.remove_agent("security").await.unwrap();
            drop(agent);
            Outcome {
                runs: runs.load(Ordering::SeqCst),
                tool_result,
                security_enforced,
            }
        })
        .await
        .expect("test task")
    })
}
