use super::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_harness::middleware::{PolicyDecision, ToolCallPolicy};
use tinyinference_llm::tool::ToolCall;
use tinytools::{
    PermissionLevel, TaskProfile, TaskRiskLevel, Tool, ToolPolicyAction, ToolPolicyDecision,
    ToolPolicySession, ToolResult,
};

struct NamedTool(&'static str);

#[async_trait]
impl Tool for NamedTool {
    fn name(&self) -> &str {
        self.0
    }

    fn description(&self) -> &str {
        "test tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
}

fn surface(allowed: &[&str]) -> LiveToolSurface {
    let tools: Vec<Box<dyn tinytools::Tool>> =
        vec![Box::new(NamedTool("alpha")), Box::new(NamedTool("beta"))];
    LiveToolSurface {
        tool_sets: vec![Arc::new(tools)],
        allowed: allowed.iter().map(|s| (*s).to_string()).collect(),
        tool_policy: None,
        has_thread: true,
        tool_hooks: Vec::new(),
        dispatch: crate::core::runtime::DispatchCtx::ambient(),
    }
}

#[test]
fn registers_only_allowed_tools() {
    let harness = assemble_live_tool_harness(surface(&["alpha"]));
    let names: Vec<_> = harness
        .tools()
        .schemas()
        .into_iter()
        .map(|schema| schema.name)
        .collect();
    assert_eq!(names, vec!["alpha"]);
}

#[test]
fn installs_the_tool_boundary_middleware() {
    let harness = assemble_live_tool_harness(surface(&["alpha", "beta"]));
    assert_eq!(harness.tools().schemas().len(), 2);
    // Approval, CLI/RPC-only and credential scrubbing wrap every call.
    assert!(harness.middleware().tool_middleware_len() >= 3);
    assert!(harness.middleware().len() >= 3);
}

#[tokio::test]
async fn runs_an_allowed_call_through_the_pipeline() {
    use tinyagents_harness::agent_loop::phases::execute_tool_batch;
    let harness = assemble_live_tool_harness(surface(&["alpha"]));
    let mut ctx = OpenHumanRunContext::new().into_tinyagents(
        tinyagents_harness::context::RunConfig::new("live-harness-test"),
    );
    let mut run = tinyagents_harness::middleware::AgentRun::new();
    let mut status = tinyagents_harness::events::HarnessRunStatus::new(
        ctx.run_id().clone(),
        tinyagents_harness::ComponentId::new("test".to_string()),
    );
    let mut messages = Vec::new();
    let outcome = execute_tool_batch(
        &harness,
        &(),
        &mut ctx,
        &mut run,
        &mut status,
        &mut messages,
        vec![tinyinference_llm::tool::ToolCall::new(
            "c1",
            "alpha",
            serde_json::json!({}),
        )],
    )
    .await
    .expect("batch runs");
    assert_eq!(outcome.results.len(), 1);
    assert!(outcome.results[0].text().contains("ok"));
}

struct ObservedTool {
    name: &'static str,
    family: Option<&'static str>,
    permission: PermissionLevel,
    external: bool,
    classified_read: bool,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for ObservedTool {
    fn name(&self) -> &str {
        self.name
    }

    fn family(&self) -> Option<&str> {
        self.family
    }

    fn description(&self) -> &str {
        "observed test tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    fn permission_level(&self) -> PermissionLevel {
        self.permission
    }

    fn external_effect(&self) -> bool {
        self.external
    }

    fn policy(&self) -> tinytools::ToolPolicy {
        if self.classified_read && self.permission <= PermissionLevel::ReadOnly && !self.external {
            tinytools::ToolPolicy::read_only()
        } else {
            tinytools::ToolPolicy::default()
        }
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult::success("effect ran"))
    }
}

struct ScriptedPolicy {
    decision: PolicyDecision,
    checks: Arc<AtomicUsize>,
}

#[async_trait]
impl ToolCallPolicy<OpenHumanRunContext> for ScriptedPolicy {
    fn name(&self) -> &str {
        "scripted_builder_policy"
    }

    async fn check(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        _call: &ToolCall,
    ) -> PolicyDecision {
        self.checks.fetch_add(1, Ordering::SeqCst);
        self.decision.clone()
    }
}

fn policy_session(tool: &str, action: ToolPolicyAction) -> ToolPolicySession {
    let mut decisions = HashMap::new();
    decisions.insert(
        tool.to_string(),
        ToolPolicyDecision {
            tool_name: tool.to_string(),
            action,
            required_permission: Some(PermissionLevel::None),
            allowed_permission: PermissionLevel::Dangerous,
        },
    );
    ToolPolicySession {
        profile: TaskProfile {
            agent_id: "live-test".into(),
            channel: "voice".into(),
            entrypoint: "live".into(),
            risk_level: TaskRiskLevel::Low,
            allowed_permission: PermissionLevel::Dangerous,
        },
        capabilities: Vec::new(),
        allowed_tool_names: Default::default(),
        blocked_tool_names: Default::default(),
        hidden_tool_names: Default::default(),
        decisions,
    }
}

fn governed_surface(
    name: &'static str,
    family: Option<&'static str>,
    permission: PermissionLevel,
    external: bool,
    action: ToolPolicyAction,
    builder_decision: PolicyDecision,
) -> (LiveToolSurface, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    governed_surface_with_classification(
        name,
        family,
        permission,
        external,
        action,
        builder_decision,
        true,
    )
}

fn governed_surface_with_classification(
    name: &'static str,
    family: Option<&'static str>,
    permission: PermissionLevel,
    external: bool,
    action: ToolPolicyAction,
    builder_decision: PolicyDecision,
    classified_read: bool,
) -> (LiveToolSurface, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let checks = Arc::new(AtomicUsize::new(0));
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(ObservedTool {
        name,
        family,
        permission,
        external,
        classified_read,
        calls: calls.clone(),
    })];
    (
        LiveToolSurface {
            tool_sets: vec![Arc::new(tools)],
            allowed: [name.to_string()].into_iter().collect(),
            tool_policy: Some(ToolPolicyEnforcement {
                policy: Arc::new(ScriptedPolicy {
                    decision: builder_decision,
                    checks: checks.clone(),
                }),
                session: policy_session(name, action),
                channel: "voice".into(),
                agent_definition_id: "live-test".into(),
            }),
            has_thread: true,
            tool_hooks: Vec::new(),
            dispatch: crate::core::runtime::DispatchCtx::ambient(),
        },
        calls,
        checks,
    )
}

fn test_core(desktop_approvals_enabled: bool) -> Arc<crate::core::runtime::CoreContext> {
    let mut config = crate::config::Config::default();
    config.desktop.approvals_enabled = desktop_approvals_enabled;
    crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config,
    )
}

async fn run_live_tool(
    harness: &AgentHarness<(), OpenHumanRunContext>,
    name: &'static str,
    core: Arc<crate::core::runtime::CoreContext>,
) -> String {
    use tinyagents_harness::agent_loop::phases::execute_tool_batch;
    let mut ctx = OpenHumanRunContext::new()
        .with_core(Some(core))
        .into_tinyagents(RunConfig::new("live-security-gate"));
    let mut run = tinyagents_harness::middleware::AgentRun::new();
    let mut status = tinyagents_harness::events::HarnessRunStatus::new(
        ctx.run_id().clone(),
        tinyagents_harness::ComponentId::new("test"),
    );
    let mut messages = Vec::new();
    let outcome = execute_tool_batch(
        harness,
        &(),
        &mut ctx,
        &mut run,
        &mut status,
        &mut messages,
        vec![ToolCall::new("c1", name, serde_json::json!({}))],
    )
    .await
    .expect("tool batch runs");
    outcome
        .results
        .into_iter()
        .next()
        .expect("one tool result")
        .text()
}

#[tokio::test]
async fn builder_policy_deny_blocks_live_execution_through_the_installed_gate() {
    let (surface, calls, checks) = governed_surface(
        "inert_observed",
        None,
        PermissionLevel::None,
        false,
        ToolPolicyAction::Allow,
        PolicyDecision::deny("builder policy refused"),
    );
    let harness = assemble_live_tool_harness(surface);
    let result = run_live_tool(&harness, "inert_observed", test_core(true)).await;
    assert!(result.contains("builder policy refused"), "{result}");
    assert_eq!(calls.load(Ordering::SeqCst), 0, "denied tool had no effect");
    assert_eq!(checks.load(Ordering::SeqCst), 1, "the callback runs once");
}

#[tokio::test]
async fn session_deny_blocks_live_execution_even_when_module_would_allow() {
    let (surface, calls, _) = governed_surface(
        "inert_observed",
        None,
        PermissionLevel::None,
        false,
        ToolPolicyAction::Deny,
        PolicyDecision::Allow,
    );
    let harness = assemble_live_tool_harness(surface);
    let result = run_live_tool(&harness, "inert_observed", test_core(true)).await;
    assert!(!result.contains("effect ran"), "{result}");
    assert_eq!(calls.load(Ordering::SeqCst), 0, "denied tool had no effect");
}

#[tokio::test]
async fn session_hidden_tool_cannot_run_from_a_direct_live_call() {
    let (surface, calls, _) = governed_surface(
        "inert_observed",
        None,
        PermissionLevel::None,
        false,
        ToolPolicyAction::HideFromPrompt,
        PolicyDecision::Allow,
    );
    let harness = assemble_live_tool_harness(surface);
    let result = run_live_tool(&harness, "inert_observed", test_core(true)).await;
    assert!(!result.contains("effect ran"), "{result}");
    assert_eq!(calls.load(Ordering::SeqCst), 0, "hidden tool had no effect");
}

#[tokio::test]
async fn desktop_approvals_off_does_not_waive_live_session_approval() {
    let (surface, calls, _) = governed_surface(
        "desktop_launch",
        Some("desktop"),
        PermissionLevel::None,
        false,
        ToolPolicyAction::RequireApproval,
        PolicyDecision::Allow,
    );
    let harness = assemble_live_tool_harness(surface);
    let result = run_live_tool(&harness, "desktop_launch", test_core(false)).await;
    assert!(!result.contains("effect ran"), "{result}");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "approval-gated tool had no effect"
    );
}

#[tokio::test]
async fn builder_approval_is_resolved_once_and_fails_closed_without_a_gate() {
    let (surface, calls, checks) = governed_surface(
        "inert_observed",
        None,
        PermissionLevel::None,
        false,
        ToolPolicyAction::Allow,
        PolicyDecision::require_approval("builder requires approval"),
    );
    let harness = assemble_live_tool_harness(surface);
    let result = run_live_tool(&harness, "inert_observed", test_core(true)).await;
    assert!(!result.contains("effect ran"), "{result}");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "unapproved tool had no effect"
    );
    assert_eq!(
        checks.load(Ordering::SeqCst),
        1,
        "one builder decision is evaluated"
    );
}

#[tokio::test]
async fn desktop_approvals_off_waives_only_builder_callback_approval() {
    let (surface, calls, checks) = governed_surface(
        "desktop_launch",
        Some("desktop"),
        PermissionLevel::None,
        false,
        ToolPolicyAction::Allow,
        PolicyDecision::require_approval("builder requires approval"),
    );
    let harness = assemble_live_tool_harness(surface);
    // The live harness also scrubs tool output. With no native test module
    // armed, that scrubber withholds the result after execution, so the tool
    // call counter is the assertion for this builder-policy behavior.
    run_live_tool(&harness, "desktop_launch", test_core(false)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1, "allowed tool runs once");
    assert_eq!(
        checks.load(Ordering::SeqCst),
        1,
        "one builder decision is evaluated"
    );
}

#[cfg(not(feature = "security-module"))]
#[tokio::test]
async fn missing_module_fails_closed_for_external_live_tools_when_approvals_are_off() {
    let (surface, calls, _) = governed_surface(
        "desktop_launch",
        Some("desktop"),
        PermissionLevel::Write,
        true,
        ToolPolicyAction::Allow,
        PolicyDecision::Allow,
    );
    let harness = assemble_live_tool_harness(surface);
    let result = run_live_tool(&harness, "desktop_launch", test_core(false)).await;
    assert!(
        !result.contains("effect ran"),
        "external effect ran without module: {result}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "external side effect is not invoked"
    );
}

#[cfg(not(feature = "security-module"))]
#[tokio::test]
async fn missing_module_keeps_inert_live_reads_available() {
    let (surface, calls, _) = governed_surface(
        "inert_observed",
        None,
        PermissionLevel::ReadOnly,
        false,
        ToolPolicyAction::Allow,
        PolicyDecision::Allow,
    );
    let harness = assemble_live_tool_harness(surface);
    // Without the module, credential scrubbing withholds the result even
    // though the explicitly inert read is allowed to execute.
    run_live_tool(&harness, "inert_observed", test_core(false)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1, "inert read runs once");
}

#[cfg(not(feature = "security-module"))]
#[tokio::test]
async fn missing_module_denies_tools_without_an_explicit_inert_declaration() {
    let (surface, calls, _) = governed_surface_with_classification(
        "unclassified_observed",
        None,
        PermissionLevel::None,
        false,
        ToolPolicyAction::Allow,
        PolicyDecision::Allow,
        false,
    );
    let harness = assemble_live_tool_harness(surface);
    let result = run_live_tool(&harness, "unclassified_observed", test_core(false)).await;
    assert!(
        !result.contains("effect ran"),
        "unclassified tool ran: {result}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
