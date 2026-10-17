//! A host-only session: the host's belt is the whole belt.
//!
//! A library host that hands an agent read-only tools over untrusted input
//! (a PR diff) must be able to say "these tools and nothing else". These
//! assert the three halves of that: nothing config-derived is registered,
//! nothing outside the host's names is admitted, and the definition the
//! session runs under can neither delegate nor act.

use super::*;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use std::sync::Arc;
use tinyagents_definition::{SandboxMode, ToolScope};
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_harness::middleware::{PolicyDecision, ToolCallPolicy};
use tinyinference_llm::tool::ToolCall;

#[derive(Debug)]
struct Marker(&'static str);

#[async_trait::async_trait]
impl tinytools::Tool for Marker {
    fn name(&self) -> &str {
        self.0
    }

    fn description(&self) -> &str {
        "a host marker"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
        Ok(tinytools::ToolResult::success("ok"))
    }
}

fn host_belt() -> crate::agent::HostTools {
    Arc::new(|_| {
        crate::agent::HostTurnTools::advertised(vec![
            Box::new(Marker("read_file")),
            Box::new(Marker("find_callers")),
        ])
    })
}

fn sorted(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut names: Vec<String> = names.into_iter().collect();
    names.sort();
    names
}

fn request(tool: &str) -> (RunContext<OpenHumanRunContext>, ToolCall) {
    (
        RunContext::new(RunConfig::new("host-only-test"), OpenHumanRunContext::new()),
        ToolCall::new("call-1", tool, serde_json::json!({})),
    )
}

/// The orchestrator's definition is the embed default, and the widest one:
/// every registered tool, delegation and memory. Host-only must narrow it.
fn orchestrator_def() -> crate::agent::harness::host_definition::AgentDefinition {
    let mut def = crate::agent::harness::host_definition::AgentDefinitionRegistry::builtins_only()
        .get("orchestrator")
        .cloned()
        .expect("built-in orchestrator");
    def.id = "reviewer".to_string();
    def.tools = ToolScope::Wildcard;
    def
}

#[test]
fn a_host_only_session_registers_and_advertises_only_the_host_belt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    let agent = crate::agent::OpenHumanSessionHost::from_config_host_only(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &orchestrator_def(),
        Some(&host_belt()),
        None,
    )
    .expect("build a host-only session");

    let expected = vec!["find_callers".to_string(), "read_file".to_string()];
    assert_eq!(
        sorted(agent.all_tool_refs().iter().map(|t| t.name().to_string())),
        expected,
        "nothing config-derived or synthesised may be registered"
    );
    assert_eq!(
        sorted(
            agent
                .visible_tool_specs_arc()
                .iter()
                .map(|spec| spec.name.clone())
        ),
        expected
    );
}

#[test]
fn a_host_only_session_without_a_belt_has_no_tools() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    let agent = crate::agent::OpenHumanSessionHost::from_config_host_only(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &orchestrator_def(),
        None,
        None,
    )
    .expect("build a host-only session");

    assert!(agent.all_tool_refs().is_empty());
    assert!(agent.visible_tool_specs_arc().is_empty());
}

#[test]
fn a_host_only_session_runs_a_definition_that_cannot_delegate_or_act() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    let agent = crate::agent::OpenHumanSessionHost::from_config_host_only(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &orchestrator_def(),
        Some(&host_belt()),
        None,
    )
    .expect("build a host-only session");

    let def = agent.resolved_definition().expect("session definition");
    assert!(def.subagents.is_empty(), "no delegation surface");
    assert!(def.deferred_tools.is_empty());
    assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
    assert!(def.extensions.omit_memory_context);
}

#[tokio::test]
async fn the_host_only_policy_admits_only_host_names() {
    let policy = super::super::host_only::HostOnlyToolPolicy::new(
        ["read_file".to_string()].into_iter().collect(),
        None,
    );

    assert_eq!(
        {
            let (ctx, call) = request("read_file");
            policy.check(&ctx, &call).await
        },
        PolicyDecision::Allow
    );
    for refused in ["shell", "write_file", "spawn_subagent", "memory_store"] {
        assert!(
            matches!(
                {
                    let (ctx, call) = request(refused);
                    policy.check(&ctx, &call).await
                },
                PolicyDecision::Deny { .. }
            ),
            "{refused} must be denied"
        );
    }
}

#[tokio::test]
async fn the_host_only_policy_still_consults_the_hosts_own_gate() {
    struct DenyAll;
    #[async_trait::async_trait]
    impl ToolCallPolicy<OpenHumanRunContext> for DenyAll {
        fn name(&self) -> &str {
            "deny_all"
        }
        async fn check(&self, _: &RunContext<OpenHumanRunContext>, _: &ToolCall) -> PolicyDecision {
            PolicyDecision::deny("host says no")
        }
    }
    let policy = super::super::host_only::HostOnlyToolPolicy::new(
        ["read_file".to_string()].into_iter().collect(),
        Some(Arc::new(DenyAll)),
    );

    assert!(matches!(
        {
            let (ctx, call) = request("read_file");
            policy.check(&ctx, &call).await
        },
        PolicyDecision::Deny { .. }
    ));
}

#[test]
fn only_a_host_only_session_accepts_untrusted_input() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let mut host_only = crate::agent::OpenHumanSessionHost::from_config_host_only(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &orchestrator_def(),
        Some(&host_belt()),
        None,
    )
    .expect("host-only session");
    let mut normal = crate::agent::OpenHumanSessionHost::from_config_with_host_tools(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &orchestrator_def(),
        &host_belt(),
        None,
    )
    .expect("normal session");

    assert!(host_only.set_untrusted_input(true).is_ok());
    assert!(normal.set_untrusted_input(true).is_err());
    assert!(normal.set_untrusted_input(false).is_ok());
}

/// The registry already keeps every built-in off a host-only belt; this is
/// the layer behind it. The gate the session will enforce at dispatch must
/// refuse a built-in name even if something registered one.
#[tokio::test]
async fn a_host_only_session_enforces_the_deny_by_default_gate() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let agent = crate::agent::OpenHumanSessionHost::from_config_host_only(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &orchestrator_def(),
        Some(&host_belt()),
        None,
    )
    .expect("host-only session");

    assert_eq!(agent.tool_policy.name(), "host_only");
    assert_eq!(
        {
            let (ctx, call) = request("read_file");
            agent.tool_policy.check(&ctx, &call).await
        },
        PolicyDecision::Allow
    );
    for built_in in ["shell", "write_file", "spawn_subagent", "tool_search"] {
        assert!(
            matches!(
                {
                    let (ctx, call) = request(built_in);
                    agent.tool_policy.check(&ctx, &call).await
                },
                PolicyDecision::Deny { .. }
            ),
            "{built_in} must be refused by the session's gate"
        );
    }
}
