use super::*;
use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_harness::middleware::Middleware;
use tinyinference_llm::tool::ToolCall as TaToolCall;

use tinytools::ToolPolicyEngine;
use tinytools::{PermissionLevel, Tool, ToolResult};

/// `skill_registry_search` with a chosen permission level.
struct PackedFake(PermissionLevel);

#[async_trait]
impl Tool for PackedFake {
    fn name(&self) -> &str {
        "skill_registry_search"
    }

    fn description(&self) -> &str {
        "fake"
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object" })
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }

    fn permission_level(&self) -> PermissionLevel {
        self.0
    }
}

/// A session in which `skill_registry_search` is withheld behind its pack
/// (hidden from the prompt, `use_skill` visible). `ceiling` is the channel's
/// permission; `None` means no channel policy is configured.
fn session(required: PermissionLevel, ceiling: Option<&str>) -> ToolPolicySession {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(PackedFake(required))];
    let permissions: HashMap<String, String> = ceiling
        .map(|c| ("web".to_string(), c.to_string()))
        .into_iter()
        .collect();
    ToolPolicyEngine::build_session(
        "orchestrator",
        "web",
        "chat",
        &permissions,
        &tools,
        &HashSet::from(["use_skill".to_string()]),
    )
}

fn open_session() -> Option<ToolPolicySession> {
    Some(session(PermissionLevel::ReadOnly, None))
}

type Mw = PackedToolRouteMiddleware<OpenHumanRunContext, SessionPackRouter>;

fn middleware(registered: &[&str], session: Option<ToolPolicySession>) -> Mw {
    packed_tool_route_middleware(registered.iter().map(|n| n.to_string()), session)
}

/// Run `before_tool`; `true` when the call was rewritten.
fn route(mw: &Mw, call: &mut TaToolCall) -> bool {
    let before = call.clone();
    let mut ctx = RunContext::new(RunConfig::new("route-test"), OpenHumanRunContext::new());
    futures::executor::block_on(mw.before_tool(&mut ctx, &(), call)).unwrap();
    *call != before
}

#[test]
fn a_bare_packed_call_becomes_the_use_skill_call_that_reaches_it() {
    let mw = middleware(&["use_skill", "shell"], open_session());
    let mut call = TaToolCall::new("c1", "skill_registry_search", json!({ "query": "x" }));

    assert!(route(&mw, &mut call));
    assert_eq!(call.name, "use_skill");
    assert_eq!(
        call.arguments,
        json!({ "skill": "skills", "tool": "skill_registry_search", "args": { "query": "x" } })
    );
}

#[test]
fn calls_the_crate_should_answer_are_left_alone() {
    let unparseable = TaToolCall {
        invalid: Some("bad json".into()),
        ..TaToolCall::new("c1", "skill_registry_search", json!("{query"))
    };
    let cases = [
        (
            middleware(&["use_skill"], open_session()),
            TaToolCall::new("c1", "made_up_tool", json!({})),
            "a name no pack owns",
        ),
        (
            middleware(&["use_skill", "file_read"], open_session()),
            TaToolCall::new("c1", "file_read", json!({})),
            "a registered name",
        ),
        (
            middleware(&["shell"], open_session()),
            TaToolCall::new("c1", "skill_registry_search", json!({})),
            "a turn without use_skill",
        ),
        (
            middleware(&["use_skill"], None),
            TaToolCall::new("c1", "skill_registry_search", json!({})),
            "a turn without a tool-policy session (sub-agent, channel/CLI)",
        ),
        (
            middleware(
                &["use_skill"],
                Some(session(PermissionLevel::Write, Some("read"))),
            ),
            TaToolCall::new("c1", "skill_registry_search", json!({})),
            "a tool over the channel ceiling",
        ),
        (
            middleware(&["use_skill"], open_session()),
            TaToolCall::new("c1", "skill_registry_search", json!(["x"])),
            "non-object arguments",
        ),
        (
            middleware(&["use_skill"], open_session()),
            unparseable,
            "unparseable arguments",
        ),
    ];
    for (mw, mut call, why) in cases {
        let before = call.clone();
        assert!(!route(&mw, &mut call), "{why}: must not route");
        assert_eq!(call, before, "{why}: call must be untouched");
    }
}
