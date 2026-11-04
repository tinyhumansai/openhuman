//! TinySecurity is admitted once per process and keeps the secret scope it was
//! admitted with. An embedded [`Runtime`] owns a fresh scratch config directory,
//! so a runtime built after an earlier one has been released derives a
//! different scope from its own config. It must still reach the admitted
//! module, through both of its paths:
//!
//! - the async client: its prompt is screened and passes, rather than being
//!   held for security review because the module "belongs" to a runtime that
//!   is gone;
//! - the synchronous adapters: its tool result is scrubbed and reaches the
//!   model, rather than being replaced by the redaction placeholder.
//!
//! Needs the TinySecurity release on `OPENHUMAN_MODULE_PATH` (or in the module
//! release cache), so it is ignored by default and run by
//! `scripts/ci/security-native-fixture.sh`.

mod common;

use std::sync::Arc;

use common::{
    chat_completion, chat_requests, offline_config, route, runtime, scripted_provider,
    stub_backend, tool_call_completion, tool_results,
};
use openhuman_embed::{
    AgentDefinitionSpec, AgentSpec, HostTurnTools, Runtime, Tool, ToolScopeSpec, Workspace,
};
use serde_json::{json, Value};
use tinyagents_session::port::InMemorySessionStores;

/// A plain request the prompt screen lets through.
const BENIGN: &str = "What is the capital of France?";

/// What the lookup tool returns; nothing in it is a credential.
const LOOKUP: &str = "capital of France: Paris";

/// A read-only host tool with a fixed answer.
struct Lookup;

#[async_trait::async_trait]
impl Tool for Lookup {
    fn name(&self) -> &str {
        "lookup"
    }
    fn description(&self) -> &str {
        "Look up a fact"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn permission_level(&self) -> tinytools::PermissionLevel {
        tinytools::PermissionLevel::ReadOnly
    }
    async fn execute(&self, _: Value) -> anyhow::Result<tinytools::ToolResult> {
        Ok(tinytools::ToolResult::success(LOOKUP))
    }
}

/// Build a runtime with the prompt screen enforced, run one turn whose model
/// calls `lookup` and then answers, and release the runtime. Returns the tool
/// result the model saw, or why the turn failed.
async fn screened_turn(name: &str) -> Result<String, String> {
    let backend = stub_backend().await;
    let model = scripted_provider(
        vec![
            tool_call_completion("lookup", "{}"),
            chat_completion("Paris"),
        ],
        "Paris",
    )
    .await;
    let runtime = Runtime::builder()
        .config(offline_config())
        .workspace(Workspace::Stateless)
        .session_store(Arc::new(InMemorySessionStores::new()))
        .backend_url(backend.uri())
        .build()
        .await
        .map_err(|error| format!("{name} runtime: {error}"))?;
    assert!(
        runtime.capabilities().configuration.security_enforced,
        "{name} runtime enforces security"
    );
    let agent = runtime
        .agent(
            AgentSpec::new(name)
                .provider(route(&model, "fixture"))
                .definition(
                    AgentDefinitionSpec::new()
                        .bare_prompt("Use lookup then answer")
                        .tools(ToolScopeSpec::HostOnly),
                )
                .tools(|_| Box::pin(async { HostTurnTools::advertised(vec![Box::new(Lookup)]) })),
        )
        .map_err(|error| format!("{name} agent: {error}"))?;
    let outcome = agent.turn(BENIGN).send().await;
    let requests = chat_requests(&model).await;
    runtime.remove_agent(name).await.expect("remove agent");
    drop(agent);
    drop(runtime);
    let outcome = outcome.map_err(|error| format!("{name} turn: {error:?}"))?;
    if requests.len() < 2 {
        return Err(format!(
            "{name} turn never returned a tool result to the model ({} requests): {}",
            requests.len(),
            outcome.reply
        ));
    }
    Ok(tool_results(&requests[1]))
}

#[test]
#[ignore = "needs the TinySecurity release; run by scripts/ci/security-native-fixture.sh"]
fn a_runtime_built_after_another_is_released_still_screens_its_prompts() {
    runtime().block_on(async {
        tokio::spawn(async {
            for name in ["first", "second"] {
                let outcome = screened_turn(name).await;
                eprintln!("{name} runtime: {outcome:?}");
                let tool_result = outcome.unwrap_or_else(|error| panic!("{error}"));
                assert!(
                    tool_result.contains(LOOKUP),
                    "{name} runtime's tool result reached the model scrubbed, not \
                     replaced: {tool_result}"
                );
            }
        })
        .await
        .expect("test task");
    });
}
