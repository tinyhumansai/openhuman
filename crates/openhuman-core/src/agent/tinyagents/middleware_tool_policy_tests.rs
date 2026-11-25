use super::*;
use crate::agent::tinyagents::middleware::tool_policy::AllowAllToolPolicy;
use crate::agent::tinyagents::middleware::tool_policy::ToolPolicyMiddleware;

// ── the listing must agree with the gate (#workflow-routing) ────────────────
//
// End to end at the only layer that holds both halves: the session allowlist
// that `use_skill` is gated on, and the tool set the listing is rendered from.
// The unit tests in `tools/toolpacks/toolpacks_tests.rs` pin the filter itself;
// these pin that the middleware feeds it the *session's* answer and resolves
// the route out of the session's own delegation tools.

use crate::tools::toolpacks::{append_pack_tools, bind_pack_registry};
use tinyagents_harness_tools::skills::USE_SKILL;
use tinytools::{PermissionLevel, ToolResult};
use tinytools::{
    TaskProfile, TaskRiskLevel, ToolCapability, ToolPolicyAction, ToolPolicyDecision,
    ToolPolicySession,
};

struct RoutingFakeTool(&'static str);

#[async_trait]
impl tinytools::Tool for RoutingFakeTool {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "fake"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        if self.0 == "spawn_subagent" {
            json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "enum": ["workflow_builder", "flow_discovery"]
                    }
                }
            })
        } else {
            json!({ "type": "object" })
        }
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
}

fn allow(name: &str) -> (String, ToolPolicyDecision) {
    (
        name.to_string(),
        ToolPolicyDecision {
            tool_name: name.to_string(),
            action: ToolPolicyAction::Allow,
            required_permission: None,
            allowed_permission: PermissionLevel::Dangerous,
        },
    )
}

/// A non-owner session: it holds the whole `workflows` pack in its registry and
/// the canonical `spawn_subagent` route to its owners. Anything not named here is denied,
/// because `decision_for` defaults to `Deny`.
fn non_owner_middleware() -> ToolPolicyMiddleware {
    non_owner_middleware_at(PermissionLevel::Dangerous)
}

/// The same session at a chosen ceiling, so a test can sit a channel BELOW the
/// pack's permission ceiling — where `use_skill` reports the pack-wide maximum
/// for an inner name it cannot resolve (#6302).
fn non_owner_middleware_at(allowed: PermissionLevel) -> ToolPolicyMiddleware {
    let mut tools: Vec<Box<dyn tinytools::Tool>> = vec![
        Box::new(RoutingFakeTool("spawn_subagent")),
        Box::new(RoutingFakeTool("propose_workflow")),
    ];
    let pack_handle = append_pack_tools(&mut tools);
    let tools = Arc::new(tools);
    bind_pack_registry(&pack_handle, &tools);

    let session = ToolPolicySession {
        profile: TaskProfile {
            agent_id: "orchestrator".to_string(),
            channel: "web_chat".to_string(),
            entrypoint: "chat".to_string(),
            risk_level: TaskRiskLevel::Low,
            allowed_permission: allowed,
        },
        capabilities: vec![ToolCapability {
            name: "spawn_subagent".to_string(),
            required_permission: PermissionLevel::ReadOnly,
        }],
        allowed_tool_names: ["spawn_subagent", USE_SKILL]
            .into_iter()
            .map(str::to_string)
            .collect(),
        blocked_tool_names: ["propose_workflow"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        hidden_tool_names: Default::default(),
        decisions: [allow("spawn_subagent"), allow(USE_SKILL)]
            .into_iter()
            .collect(),
    };

    ToolPolicyMiddleware::new(
        Arc::new(AllowAllToolPolicy),
        session,
        vec![tools],
        "web_chat".to_string(),
    )
}

#[test]
fn desktop_approval_bypass_skips_only_require_approval_channel_verdict() {
    fn middleware(action: ToolPolicyAction) -> ToolPolicyMiddleware {
        let name = "desktop_goal";
        let session = ToolPolicySession {
            profile: TaskProfile {
                agent_id: "orchestrator".to_owned(),
                channel: "web_chat".to_owned(),
                entrypoint: "chat".to_owned(),
                risk_level: TaskRiskLevel::Low,
                allowed_permission: PermissionLevel::Dangerous,
            },
            capabilities: Vec::new(),
            allowed_tool_names: Default::default(),
            blocked_tool_names: Default::default(),
            hidden_tool_names: Default::default(),
            decisions: [(
                name.to_owned(),
                ToolPolicyDecision {
                    tool_name: name.to_owned(),
                    action,
                    required_permission: Some(PermissionLevel::Write),
                    allowed_permission: PermissionLevel::Dangerous,
                },
            )]
            .into_iter()
            .collect(),
        };
        ToolPolicyMiddleware::new(
            Arc::new(AllowAllToolPolicy),
            session,
            vec![Arc::new(vec![Box::new(RoutingFakeTool(name))])],
            "web_chat".to_owned(),
        )
    }
    let approval_middleware = middleware(ToolPolicyAction::RequireApproval);
    let request = call("desktop_goal", json!({"app":"TextEdit","goal":"type"}));
    assert!(approval_middleware
        .channel_permission_block(&request, false)
        .is_some());
    assert!(approval_middleware
        .channel_permission_block(&request, true)
        .is_none());
    assert!(middleware(ToolPolicyAction::Deny)
        .channel_permission_block(&request, true)
        .is_some());
}

fn call(name: &str, args: serde_json::Value) -> TaToolCall {
    TaToolCall {
        id: "call-1".to_string(),
        name: name.to_string(),
        arguments: args,
        invalid: None,
    }
}

/// A non-owner cannot call workflow pack tools directly; it must be sent to an
/// owner through the same spawn route advertised by the orchestrator.
#[tokio::test]
async fn a_use_skill_listing_hides_what_this_session_cannot_call_and_names_the_route() {
    let mw = non_owner_middleware();
    let result = mw
        .render_skill_for_session(&call(USE_SKILL, json!({ "skill": "workflows" })))
        .expect("a use_skill call naming a skill and no tool renders here");

    assert!(
        !result_text(&result).contains("propose_workflow"),
        "a non-owner must not be offered a tool the gate will refuse:\n{}",
        result_text(&result)
    );
    assert!(
        result_text(&result).contains("has no tools available in this session"),
        "the listing must not invent a workflow tool for this non-owner:\n{}",
        result_text(&result)
    );
    assert!(
        result_text(&result).contains("Call `spawn_subagent` to delegate")
            && (result_text(&result).contains("workflow_builder")
                || result_text(&result).contains("flow_discovery")),
        "the empty listing must name the callable owner hand-off:\n{}",
        result_text(&result)
    );
    assert!(!result_text(&result).contains("## `propose_workflow`"));
}

/// The middleware renders the *disclosure* half only. A `use_skill` call that
/// names a `tool` is the execution half: it has already passed
/// `channel_permission_block`, and rendering a listing in its place would turn
/// every packed-tool call into a menu. That call must fall through to the
/// tool's own `execute`.
#[tokio::test]
async fn a_use_skill_call_that_names_a_tool_is_not_rendered_as_a_listing() {
    let mw = non_owner_middleware();
    assert!(
        mw.render_skill_for_session(&call(
            USE_SKILL,
            json!({ "skill": "workflows", "tool": "propose_workflow" }),
        ))
        .is_none(),
        "the execution half must reach the tool, not the listing"
    );
    // An empty `tool` is the disclosure half — the same rule `named_tool` applies.
    assert!(
        mw.render_skill_for_session(&call(
            USE_SKILL,
            json!({ "skill": "workflows", "tool": "" }),
        ))
        .is_some(),
        "an empty tool name is a listing request, not a call"
    );
}

/// And when it calls the denied tool anyway, the refusal has to carry the way
/// out — resolved from a delegate this session actually holds.
#[tokio::test]
async fn a_use_skill_denial_names_a_delegate_this_session_can_call() {
    let mw = non_owner_middleware();
    let message = mw
        .channel_permission_block(
            &call(
                "use_skill",
                json!({ "skill": "workflows", "tool": "propose_workflow" }),
            ),
            false,
        )
        .expect("propose_workflow is denied for a non-owner");

    assert!(
        message.contains("not allowed in the current session"),
        "the denial itself must survive: {message}"
    );
    assert!(
        message.contains("Call `spawn_subagent` to delegate")
            && message.contains("owned by `workflow_builder`"),
        "the denial must name the callable spawn tool and owning specialist: {message}"
    );
}

/// The route is only trustworthy because it is read off the session's own tool
/// set. A session with no delegate to `workflow_builder` must fall back to
/// naming the owning agents rather than inventing a call it cannot make.
#[tokio::test]
async fn a_session_without_the_delegate_is_not_told_to_call_it() {
    let mut tools: Vec<Box<dyn tinytools::Tool>> =
        vec![Box::new(RoutingFakeTool("propose_workflow"))];
    let pack_handle = append_pack_tools(&mut tools);
    let tools = Arc::new(tools);
    bind_pack_registry(&pack_handle, &tools);

    let session = ToolPolicySession {
        profile: TaskProfile {
            agent_id: "orchestrator".to_string(),
            channel: "web_chat".to_string(),
            entrypoint: "chat".to_string(),
            risk_level: TaskRiskLevel::Low,
            allowed_permission: PermissionLevel::Dangerous,
        },
        capabilities: vec![],
        allowed_tool_names: [USE_SKILL].into_iter().map(str::to_string).collect(),
        blocked_tool_names: ["propose_workflow"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        hidden_tool_names: Default::default(),
        decisions: [allow(USE_SKILL)].into_iter().collect(),
    };
    let mw = ToolPolicyMiddleware::new(
        Arc::new(AllowAllToolPolicy),
        session,
        vec![tools],
        "web_chat".to_string(),
    );

    let message = mw
        .channel_permission_block(
            &call(
                "use_skill",
                json!({ "skill": "workflows", "tool": "propose_workflow" }),
            ),
            false,
        )
        .expect("propose_workflow is denied");

    assert!(
        message.contains("workflow_builder"),
        "with no delegate to call, name the owning agent: {message}"
    );
    assert!(
        !message.contains("Call `spawn_subagent`"),
        "must not instruct a call this session cannot make: {message}"
    );
}

/// **Pre-existing, and larger than the bug this branch was opened for.**
///
/// `channel_permission_block`'s inner `use_skill` check denies on
/// `decision.is_denied()`, which is true for `HideFromPrompt`. Every withheld
/// packed tool is `HideFromPrompt` for a non-owner — that is what "withheld"
/// means. So `use_skill`, the *only* advertised route to a withheld tool, was
/// refusing all of them. AGENTS.md's disclosure table says `Withheld` is
/// "Registered and callable: yes".
#[tokio::test]
async fn use_skill_reaches_a_withheld_packed_tool() {
    use crate::tools::toolpacks::strip_packed_from_visible;
    use tinytools::ToolPolicyEngine;

    let mut tools: Vec<Box<dyn tinytools::Tool>> = vec![Box::new(RoutingFakeTool("goal_set"))];
    let pack_handle = append_pack_tools(&mut tools);
    let tools = Arc::new(tools);
    bind_pack_registry(&pack_handle, &tools);

    // The real harness shape: seed `visible` with everything, then withhold the
    // packed names for a non-owner.
    let mut visible: std::collections::HashSet<String> =
        tools.iter().map(|t| t.name().to_string()).collect();
    strip_packed_from_visible(&mut visible, "orchestrator");
    let session = ToolPolicyEngine::build_session(
        "orchestrator",
        "web_chat",
        "chat",
        &Default::default(),
        &tools,
        &visible,
    );

    let mw = ToolPolicyMiddleware::new(
        Arc::new(AllowAllToolPolicy),
        session,
        vec![tools],
        "web_chat".to_string(),
    );

    assert!(
        mw.channel_permission_block(
            &call("use_skill", json!({ "skill": "goals", "tool": "goal_set" }),),
            false
        )
        .is_none(),
        "a withheld packed tool must stay reachable through use_skill — that is \
         the only route it has"
    );
}

/// **A route hint must use the predicate of the gate it points at.**
///
/// `route_sentence` tells the model to make a *direct* call, and the direct-call
/// gate rejects `HideFromPrompt`. So a prompt-hidden delegate is not a route:
/// naming it would send the model into the same refusal this branch exists to
/// remove, one hop later. The hint must fall back to the owning agent instead.
#[tokio::test]
async fn a_prompt_hidden_delegate_is_not_offered_as_a_direct_route() {
    use crate::tools::toolpacks::strip_packed_from_visible;
    use tinytools::ToolPolicyEngine;

    let mut tools: Vec<Box<dyn tinytools::Tool>> = vec![
        Box::new(RoutingFakeTool("propose_workflow")),
        Box::new(RoutingFakeTool("spawn_subagent")),
    ];
    let pack_handle = append_pack_tools(&mut tools);
    let tools = Arc::new(tools);
    bind_pack_registry(&pack_handle, &tools);

    // The spawn tool is withheld from this session, so even though its schema
    // can reach the workflow owners it is not a callable route.
    let mut visible: std::collections::HashSet<String> =
        tools.iter().map(|t| t.name().to_string()).collect();
    strip_packed_from_visible(&mut visible, "unrelated_agent");
    visible.remove("spawn_subagent");
    assert!(
        !visible.contains("spawn_subagent"),
        "precondition: the owner hand-off is prompt-hidden here"
    );

    let session = ToolPolicyEngine::build_session(
        "orchestrator",
        "web_chat",
        "chat",
        &Default::default(),
        &tools,
        &visible,
    );
    let mw = ToolPolicyMiddleware::new(
        Arc::new(AllowAllToolPolicy),
        session,
        vec![tools],
        "web_chat".to_string(),
    );

    let pack = crate::tools::toolpacks::pack("workflows").expect("workflows pack");
    let route = mw.route_for_pack(pack);
    assert!(
        !route.contains("Call `spawn_subagent` to delegate"),
        "a prompt-hidden delegate must not be advertised as a direct call: {route}"
    );
    assert!(
        route.contains("workflow_builder") || route.contains("flow_discovery"),
        "when the spawn route is hidden, name an owning agent without claiming a callable tool: {route}"
    );
}

/// #6302: an invented tool name inside `use_skill` is "no such tool", not a
/// permission denial. The expected text comes from the same typed value the gate
/// renders, so the two cannot drift.
#[tokio::test]
async fn an_invented_tool_name_in_a_skill_is_not_reported_as_a_denial() {
    let mw = non_owner_middleware();
    let message = mw
        .channel_permission_block(
            &call(
                "use_skill",
                json!({ "skill": "workflows", "tool": "install_workflow" }),
            ),
            false,
        )
        .expect("an invented tool must be answered, not dispatched");

    let workflows = crate::tools::toolpacks::pack("workflows").expect("workflows pack");
    assert_eq!(
        message,
        mw.no_such_pack_tool(workflows, "install_workflow").render(),
        "the gate must answer an invented name with the typed not-found message"
    );
    assert!(
        !message.contains("not allowed"),
        "an invented name must not read as a permission denial: {message}"
    );
    assert!(
        message.contains("Call `spawn_subagent` to delegate")
            && (message.contains("workflow_builder") || message.contains("flow_discovery")),
        "the answer must name what this session can call in the skill: {message}"
    );
}

/// A channel under the pack's ceiling must still get "no such tool".
///
/// `UseSkillTool::permission_level_with_args` cannot resolve an invented inner
/// name, so it reports the pack-wide CEILING. With the ceiling check running
/// first, that turned an invented name into `PermissionTooLow` — a permission
/// denial for a tool that does not exist, on exactly the restricted channels
/// where the wording matters most (#6302).
#[test]
fn an_invented_tool_name_is_not_found_even_under_the_packs_permission_ceiling() {
    let mw = non_owner_middleware_at(PermissionLevel::ReadOnly);
    let message = mw
        .channel_permission_block(
            &call(
                "use_skill",
                json!({ "skill": "workflows", "tool": "install_workflow" }),
            ),
            false,
        )
        .expect("an invented tool must be answered, not dispatched");

    assert!(
        message.starts_with(tinytools::status::NOT_FOUND_MARKER),
        "a restricted channel must still answer an invented name as not-found: {message}"
    );
    assert!(
        !message.contains("permission"),
        "existence is not a permission question: {message}"
    );
}

/// A pack member this core never registered is absent, not forbidden.
///
/// A feature gate or an embedder's `ToolGroups::Off` removes tools the static
/// pack table still lists. `pack.owns` says yes, so the call fell through to
/// the default `Deny` and read as "not allowed in the current session" — which
/// sends the model looking for permission it can never be granted, instead of
/// telling it the tool is not here.
#[test]
fn a_pack_member_this_session_never_registered_is_not_found_not_forbidden() {
    let mw = non_owner_middleware();
    // `save_workflow` is in the `workflows` pack, but this session's registry
    // holds the spawn tool and `propose_workflow`, but not `save_workflow`.
    let workflows = crate::tools::toolpacks::pack("workflows").expect("workflows pack");
    assert!(
        workflows.owns("save_workflow"),
        "the fixture depends on `save_workflow` being a real pack member"
    );

    let message = mw
        .channel_permission_block(
            &call(
                "use_skill",
                json!({ "skill": "workflows", "tool": "save_workflow" }),
            ),
            false,
        )
        .expect("an unregistered member must be answered, not dispatched");

    assert_eq!(
        message,
        mw.no_such_pack_tool(workflows, "save_workflow").render(),
        "an absent pack member gets the typed not-found message"
    );
    assert!(
        !message.contains("not allowed"),
        "absence must not read as a permission denial: {message}"
    );
}

/// Only `use_skill` renders a pack listing. Any other registered tool whose
/// arguments carry a string `skill` (an MCP schema, a stray key) must fall
/// through to its own `execute`.
#[tokio::test]
async fn a_non_use_skill_call_with_a_skill_argument_is_not_rendered_as_a_listing() {
    let mw = non_owner_middleware();
    assert!(
        mw.render_skill_for_session(&call("build_workflow", json!({ "skill": "workflows" })))
            .is_none(),
        "build_workflow was answered with a pack listing instead of executing"
    );
}
