//! End-to-end nested tool calls (`ToolExecutionContext::call_tool`) with the
//! host's enforcement in place.
//!
//! Nested calls are off in production (`RunLimits::max_nested_depth = 0`); these
//! tests turn them on and drive a real nested call through the harness to prove
//! the host fails closed when they are enabled:
//!
//! - `EmbedderToolHooksMiddleware` denies a nested call its hook denies,
//! - `OpenHumanSecurityGate` refuses a nested call on the hosted path,
//! - an allowed nested call still runs.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyagents_definition::{AgentDefinition, InMemoryDefinitionRegistry};
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_harness::host::{
    FixedModelResolver, HostCapabilities, SecurityGate, StaticContextComposer,
};
use tinyagents_harness::limits::RunLimits;
use tinyagents_harness::runtime::{AgentHarness, AgentInvocation, AgentTurnRequest, RunPolicy};
use tinyagents_harness::testkit::{text_response, tool_call_response};
use tinyagents_harness::tool::ToolExecutionContext;
use tinyinference_llm::message::Message;
use tinyinference_llm::providers::MockModel;
use tinyinference_llm::tool::ToolCall;
use tinytools::{PermissionLevel, Tool, ToolResult};

use crate::agent::hooks::{ToolHook, ToolHookContext, ToolHookDecision};
use crate::agent::tinyagents::host::{OpenHumanRunContext, OpenHumanSecurityGate};
use crate::agent::tinyagents::middleware::embedder_hooks::EmbedderToolHooksMiddleware;
use crate::security::policy::{AutonomyLevel, SecurityPolicy};

type Outcome = Result<ToolResult, String>;

/// A read-only leaf tool that records how often it ran.
struct Leaf {
    name: &'static str,
    ran: Arc<Mutex<usize>>,
}

#[async_trait]
impl Tool for Leaf {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "leaf"
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        *self.ran.lock().unwrap() += 1;
        Ok(ToolResult::success(format!("{}-out", self.name)))
    }
}

/// A tool that makes one nested call to `target` and records the outcome.
struct Caller {
    target: &'static str,
    outcome: Arc<Mutex<Option<Outcome>>>,
}

#[async_trait]
impl Tool for Caller {
    fn name(&self) -> &str {
        "caller"
    }
    fn description(&self) -> &str {
        "calls another tool"
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        unreachable!("the harness dispatches through execute_with_context")
    }
    async fn execute_with_context(
        &self,
        _args: Value,
        _options: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let harness = context
            .and_then(tinytools::ToolRunContext::host_extension)
            .and_then(|any| any.downcast_ref::<ToolExecutionContext>())
            .expect("the harness installs its context")
            .clone();
        let result = harness.call_tool(self.target, json!({})).await;
        *self.outcome.lock().unwrap() = Some(result.map_err(|error| error.to_string()));
        Ok(ToolResult::success("caller-out"))
    }
}

/// Denies one named tool, allows everything else.
struct DenyNamed(&'static str);

#[async_trait]
impl ToolHook for DenyNamed {
    fn name(&self) -> &str {
        "deny_named"
    }
    async fn before_tool(&self, context: &ToolHookContext) -> anyhow::Result<()> {
        if context.tool_name == self.0 {
            anyhow::bail!("{} is forbidden", self.0);
        }
        Ok(())
    }
    async fn after_tool(&self, _context: &ToolHookContext) -> anyhow::Result<()> {
        Ok(())
    }
    async fn before_tool_decision(&self, context: &ToolHookContext) -> ToolHookDecision {
        match self.before_tool(context).await {
            Ok(()) => ToolHookDecision::Proceed,
            Err(error) => ToolHookDecision::Deny(format!("{error:#}")),
        }
    }
}

fn nested_enabled() -> RunPolicy {
    RunPolicy {
        limits: RunLimits::default().with_max_nested_depth(1),
        ..RunPolicy::default()
    }
}

fn model() -> Arc<MockModel> {
    Arc::new(MockModel::with_responses(vec![
        tool_call_response(ToolCall::new("p1", "caller", json!({}))),
        text_response("done"),
    ]))
}

fn leaf(name: &'static str) -> (Arc<Leaf>, Arc<Mutex<usize>>) {
    let ran = Arc::new(Mutex::new(0));
    (
        Arc::new(Leaf {
            name,
            ran: ran.clone(),
        }),
        ran,
    )
}

fn run_context() -> RunContext<OpenHumanRunContext> {
    RunContext::new(RunConfig::new("nested-e2e"), OpenHumanRunContext::new())
}

/// A plain (unhosted) run whose only enforcement is the embedder hooks.
async fn run_with_hooks(target: &'static str) -> (Option<Outcome>, usize, usize) {
    let (allowed, allowed_ran) = leaf("allowed");
    let (secret, secret_ran) = leaf("secret");
    let outcome = Arc::new(Mutex::new(None));
    let mut harness: AgentHarness<(), OpenHumanRunContext> = AgentHarness::new();
    harness.register_model("mock", model());
    harness.with_policy(nested_enabled());
    harness.register_tool(allowed);
    harness.register_tool(secret);
    harness.register_tool(Arc::new(Caller {
        target,
        outcome: outcome.clone(),
    }));
    harness.push_middleware(Arc::new(EmbedderToolHooksMiddleware::new(vec![Arc::new(
        DenyNamed("secret"),
    )])));
    harness
        .invoke_in_context(&(), run_context(), vec![Message::user("go")])
        .await
        .expect("the run completes");
    let outcome = outcome.lock().unwrap().take();
    let allowed_ran = *allowed_ran.lock().unwrap();
    let secret_ran = *secret_ran.lock().unwrap();
    (outcome, allowed_ran, secret_ran)
}

#[tokio::test]
async fn a_nested_call_the_embedder_hook_denies_never_runs() {
    let (outcome, _, secret_ran) = run_with_hooks("secret").await;
    let error = outcome
        .expect("the caller attempted a nested call")
        .expect_err("the hook denial refuses the nested call");
    assert!(error.contains("secret is forbidden"), "{error}");
    assert_eq!(secret_ran, 0, "the denied tool must not execute");
}

#[tokio::test]
async fn a_nested_call_the_embedder_hook_allows_runs() {
    let (outcome, allowed_ran, _) = run_with_hooks("allowed").await;
    let result = outcome
        .expect("the caller attempted a nested call")
        .expect("an allowed nested call passes");
    assert_eq!(result.output(), "allowed-out");
    assert_eq!(allowed_ran, 1);
}

#[tokio::test]
async fn the_security_gate_refuses_a_nested_call_on_the_hosted_path() {
    let (target, target_ran) = leaf("read_file");
    let outcome = Arc::new(Mutex::new(None));
    let gate: Arc<dyn SecurityGate> = Arc::new(OpenHumanSecurityGate::new(
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            ..SecurityPolicy::default()
        }),
        // The gate resolves permission metadata by name, and an unknown tool is
        // denied, so both the parent and the nested target are registered. The
        // parent must pass for the nested call to be attempted at all.
        vec![Arc::new(
            ["caller", "read_file"]
                .into_iter()
                .map(|name| {
                    Box::new(Leaf {
                        name,
                        ran: Arc::new(Mutex::new(0)),
                    }) as Box<dyn Tool>
                })
                .collect(),
        )],
    ));
    let host = HostCapabilities::new(
        Arc::new(StaticContextComposer::empty()),
        Arc::new(InMemoryDefinitionRegistry::new(vec![AgentDefinition::new(
            "helper", "Helper", "test",
        )
        .with_tools(["caller", "read_file"])])),
        gate,
        Arc::new(FixedModelResolver::new(model())),
    );
    let mut harness: AgentHarness<(), OpenHumanRunContext> = AgentHarness::new();
    harness.with_policy(nested_enabled());
    harness.register_tool(target);
    harness.register_tool(Arc::new(Caller {
        target: "read_file",
        outcome: outcome.clone(),
    }));

    harness
        .invoke_agent(
            AgentInvocation::new(
                host,
                AgentTurnRequest::new("helper", vec![Message::user("go")]),
                run_context(),
            ),
            &(),
        )
        .await
        .expect("the run completes");

    let error = outcome
        .lock()
        .unwrap()
        .take()
        .expect("the caller attempted a nested call")
        .expect_err("the gate refuses a nested call");
    assert!(error.contains("may not call other tools"), "{error}");
    assert_eq!(
        *target_ran.lock().unwrap(),
        0,
        "the refused tool must not run"
    );
}
