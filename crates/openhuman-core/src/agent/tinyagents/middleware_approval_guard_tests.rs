use super::*;

// ── ApprovalSecurityMiddleware ──────────────────────────────────────────

#[test]
fn approval_external_effect_resolution_walks_the_tool_sets() {
    let tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![
        Box::new(FakeTool {
            name: "send_email",
            cap: None,
            external: true,
        }),
        Box::new(FakeTool {
            name: "read_file",
            cap: None,
            external: false,
        }),
    ]);
    let mw = ApprovalSecurityMiddleware::new(vec![tools]);
    assert!(mw.has_external_effect("send_email", &json!({})));
    assert!(!mw.has_external_effect("read_file", &json!({})));
    // Unknown tool defaults to no external effect (nothing to gate).
    assert!(!mw.has_external_effect("missing", &json!({})));
}

#[cfg(feature = "modules")]
#[test]
fn desktop_external_effect_keeps_metadata_but_skips_approval_when_disabled() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(desktop_external_effect_inner());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[cfg(feature = "modules")]
async fn desktop_external_effect_inner() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("config.toml"),
        "[desktop]\napprovals_enabled = false\n",
    )
    .unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", temp.path());
    }
    let tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![
        Box::new(crate::desktop::control::tools::DesktopTool::new(
            Arc::new(crate::config::Config::default()),
            crate::desktop::control::tools::DesktopToolKind::Goal,
        )),
        Box::new(FakeTool {
            name: "send_email",
            cap: None,
            external: true,
        }),
    ]);
    let middleware = ApprovalSecurityMiddleware::new(vec![tools]);
    assert!(middleware.has_external_effect("desktop_goal", &json!({})));
    assert!(
        !middleware
            .requires_approval("desktop_goal", &json!({}))
            .await
    );
    assert!(middleware.requires_approval("send_email", &json!({})).await);
    match previous {
        Some(value) => unsafe { std::env::set_var("OPENHUMAN_WORKSPACE", value) },
        None => unsafe { std::env::remove_var("OPENHUMAN_WORKSPACE") },
    }
}

#[test]
fn approval_identity_scopes_composio_dispatcher_grants_to_one_action() {
    assert_eq!(
        approval_tool_name(
            "composio_execute",
            &json!({ "tool": "  GMAIL_SEND_EMAIL  " })
        ),
        "composio_execute:GMAIL_SEND_EMAIL"
    );
    assert_eq!(
        approval_tool_name("composio_execute", &json!({ "tool": "GMAIL_DELETE_EMAIL" })),
        "composio_execute:GMAIL_DELETE_EMAIL"
    );
    assert_eq!(
        approval_tool_name("composio_execute", &json!({})),
        "composio_execute:<invalid-action>"
    );
    assert_eq!(
        approval_tool_name("send_email", &json!({ "tool": "ignored" })),
        "send_email"
    );
}

// ── issue #7649: a permission-refused call must not raise an approval card ──
//
// The approval wrap is the OUTERMOST tool middleware and the session's
// channel-permission ceiling lives on the inner `ToolPolicyMiddleware`, so
// `requires_approval` is the last chance to keep a card from firing for a call
// the permission layer will refuse regardless of the human's answer.

use crate::tools::agent_policy::{
    TaskProfile, TaskRiskLevel, ToolPolicyAction, ToolPolicyDecision, ToolPolicySession,
};
use tinyagents_harness::middleware::ApprovalResolver;
use tinytools::PermissionLevel;

/// An external-effect tool at a chosen permission level, so the ceiling branch
/// of the permission view can be exercised (`FakeTool` sits at the `ReadOnly`
/// default, which no plausible test ceiling sits below).
struct GatedTool {
    name: &'static str,
    level: PermissionLevel,
}

#[async_trait]
impl Tool for GatedTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "fake external-effect tool"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object" })
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
        Ok(tinytools::ToolResult::success("ok"))
    }
    fn external_effect_with_args(&self, _args: &serde_json::Value) -> bool {
        true
    }
    fn permission_level(&self) -> PermissionLevel {
        self.level
    }
}

/// A session snapshot with one named decision at a channel ceiling, the shape
/// `ToolPolicyEnforcement` hands both gates.
fn session_with_decision(
    name: &str,
    action: ToolPolicyAction,
    allowed: PermissionLevel,
) -> ToolPolicySession {
    ToolPolicySession {
        profile: TaskProfile {
            agent_id: "agent-under-test".to_owned(),
            channel: "web_chat".to_owned(),
            entrypoint: "chat".to_owned(),
            risk_level: TaskRiskLevel::Low,
            allowed_permission: allowed,
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
                required_permission: None,
                allowed_permission: allowed,
            },
        )]
        .into_iter()
        .collect(),
    }
}

fn approval_call(name: &str) -> TaToolCall {
    TaToolCall {
        id: "call-7649".to_owned(),
        name: name.to_owned(),
        arguments: json!({}),
        invalid: None,
    }
}

fn gated_tool_set() -> Vec<Arc<Vec<Box<dyn Tool>>>> {
    vec![Arc::new(vec![Box::new(GatedTool {
        name: "send_email",
        level: PermissionLevel::Write,
    })])]
}

/// The session denies `send_email`, so the inner `ToolPolicyMiddleware` will
/// refuse it no matter how the human answers — prompting first is the
/// approval-then-deny interruption #7649 reports.
#[tokio::test]
async fn a_session_denied_external_effect_tool_is_not_held_for_approval() {
    let ctx = ctx();
    let mw = ApprovalSecurityMiddleware::new(gated_tool_set()).with_session_permission(Some((
        session_with_decision(
            "send_email",
            ToolPolicyAction::Deny,
            PermissionLevel::Dangerous,
        ),
        "web_chat".to_owned(),
    )));
    assert!(
        !ApprovalResolver::requires_approval(&mw, &ctx, &approval_call("send_email")).await,
        "a call the session permission layer will refuse must not prompt"
    );
}

/// The same tool above the session's permission ceiling: refused
/// (`PermissionTooLow`) regardless of the answer, so no card either.
#[tokio::test]
async fn a_call_above_the_session_permission_ceiling_is_not_held_for_approval() {
    let ctx = ctx();
    let mw = ApprovalSecurityMiddleware::new(gated_tool_set()).with_session_permission(Some((
        session_with_decision(
            "send_email",
            ToolPolicyAction::Allow,
            PermissionLevel::ReadOnly,
        ),
        "web_chat".to_owned(),
    )));
    assert!(
        !ApprovalResolver::requires_approval(&mw, &ctx, &approval_call("send_email")).await,
        "a call above the channel ceiling is refused anyway; prompting is noise"
    );
}

/// Control: the same external-effect tool under a session that permits it at
/// its level still needs approval — the pre-check must not swallow prompts the
/// permission layer has no opinion on.
#[tokio::test]
async fn a_permitted_external_effect_tool_still_requires_approval() {
    let ctx = ctx();
    let mw = ApprovalSecurityMiddleware::new(gated_tool_set()).with_session_permission(Some((
        session_with_decision(
            "send_email",
            ToolPolicyAction::Allow,
            PermissionLevel::Dangerous,
        ),
        "web_chat".to_owned(),
    )));
    assert!(ApprovalResolver::requires_approval(&mw, &ctx, &approval_call("send_email")).await);
}

/// No session snapshot (CLI / sub-agent paths): behaviour unchanged — the
/// external-effect tool still prompts.
#[tokio::test]
async fn without_a_session_snapshot_the_prompt_decision_is_unchanged() {
    let ctx = ctx();
    let mw = ApprovalSecurityMiddleware::new(gated_tool_set());
    assert!(ApprovalResolver::requires_approval(&mw, &ctx, &approval_call("send_email")).await);
}
