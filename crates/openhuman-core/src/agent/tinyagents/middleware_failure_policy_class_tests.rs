//! Recovery classes for policy refusals and the read-only timeout floor.
//!
//! - A call the security policy blocked never ran, and its refusal tells the
//!   model to try a narrower, permitted alternative. Production turns (Oct
//!   2026, ~18 breaker stops) ended after that first refusal because the
//!   `policy` class had a budget of zero. A user's explicit denial and an
//!   expired approval still stop at once.
//! - The timeout floor named tools that no longer exist (`web_search`); the
//!   real names must keep their retryable timeouts from the side-effect
//!   reading alone.

#[cfg(feature = "modules")]
use super::super::failure_policy::recovery_policy_with_effect;
use super::super::repeated_failure::recovery_policy;
use super::*;
use crate::agent::tinyagents::middleware::repeated_failure::RepeatedToolFailureMiddleware;
#[cfg(feature = "modules")]
use tinyagents_harness::middleware::{call_effect, tool_sets_lookup, CallEffect};

/// The shape of a production refusal: the gate's marker, then its reason.
const SHELL_BLOCK: &str =
    "[policy-blocked] Unbounded system scan of '/' is not allowed; scope the command to the \
     project directory or a narrower path.";
const FETCH_BLOCK: &str = "[policy-blocked] Command not allowed by security policy: web_fetch";
const FILE_BLOCK: &str = "[policy-blocked] Security policy: forbidden path /etc/shadow";

async fn run_call(
    mw: &RepeatedToolFailureMiddleware,
    id: &str,
    tool: &str,
    args: serde_json::Value,
    outcome: Result<&str, &str>,
) {
    let mut call = TaToolCall::new(id, tool, args);
    mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
    let mut result = match outcome {
        Ok(text) => TaToolResult::success(text),
        Err(error) => failing_result(tool, error),
    };
    mw.after_tool(&mut ctx(), &(), &invocation(id, tool), &mut result)
        .await
        .unwrap();
}

fn breaker() -> (
    RepeatedToolFailureMiddleware,
    SteeringHandle,
    tinyagents_harness::turn_outcome::HaltSummarySlot,
) {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    (mw, handle, slot)
}

// ── policy blocks get one alternative ───────────────────────────────────────

#[test]
fn a_policy_block_allows_one_alternative_but_a_user_denial_does_not() {
    for (tool, error) in [
        ("shell", SHELL_BLOCK),
        ("web_fetch", FETCH_BLOCK),
        ("file_read", FILE_BLOCK),
    ] {
        assert_eq!(
            recovery_policy(tool, error, false),
            Some(("blocked_by_policy", 1)),
            "{tool}"
        );
    }
    // The user said no, or never answered: asking again only re-prompts them.
    assert_eq!(
        recovery_policy(
            "shell",
            "[policy-denied] The user declined this command",
            false
        ),
        Some(("policy", 0))
    );
    assert_eq!(
        recovery_policy(
            "shell",
            "[policy-denied] Approval request timed out and expired",
            false
        ),
        Some(("policy", 0))
    );
}

#[tokio::test]
async fn a_blocked_shell_command_lets_the_model_try_a_narrower_one() {
    let (mw, handle, slot) = breaker();
    run_call(
        &mw,
        "scan-1",
        "shell",
        serde_json::json!({"command": "find / -name '*.log'"}),
        Err(SHELL_BLOCK),
    )
    .await;
    assert_eq!(drain_pause_count(&handle), 0, "one refusal is not a halt");
    assert!(slot.lock().unwrap().is_none());
    let nudges = drain_nudge_messages(&mw);
    assert_eq!(nudges.len(), 1, "{nudges:?}");
    assert!(nudges[0].contains("narrower"), "{nudges:?}");
    assert!(nudges[0].contains("Unbounded system scan"), "{nudges:?}");

    // The narrower alternative runs: the blocker is cleared, so a later
    // refusal starts a fresh budget.
    run_call(
        &mw,
        "scan-2",
        "shell",
        serde_json::json!({"command": "find ./logs -name '*.log'"}),
        Ok("logs/app.log"),
    )
    .await;
    run_call(
        &mw,
        "scan-3",
        "shell",
        serde_json::json!({"command": "du -sh /"}),
        Err(SHELL_BLOCK),
    )
    .await;
    assert_eq!(drain_pause_count(&handle), 0);
    assert!(slot.lock().unwrap().is_none());
}

#[tokio::test]
async fn a_second_consecutive_policy_block_stops_the_turn() {
    let (mw, handle, slot) = breaker();
    for (id, command) in [("scan-1", "find / -name x"), ("scan-2", "ls -R /")] {
        run_call(
            &mw,
            id,
            "shell",
            serde_json::json!({"command": command}),
            Err(SHELL_BLOCK),
        )
        .await;
    }
    assert_eq!(drain_pause_count(&handle), 1);
    let summary = slot.lock().unwrap().clone().unwrap();
    assert!(summary.contains("blocked_by_policy"), "{summary}");
}

#[tokio::test]
async fn blocks_on_different_tools_are_budgeted_separately() {
    let (mw, handle, slot) = breaker();
    run_call(
        &mw,
        "f-1",
        "web_fetch",
        serde_json::json!({"url": "https://blocked.test/a"}),
        Err(FETCH_BLOCK),
    )
    .await;
    run_call(
        &mw,
        "r-1",
        "file_read",
        serde_json::json!({"path": "/etc/shadow"}),
        Err(FILE_BLOCK),
    )
    .await;
    assert_eq!(drain_pause_count(&handle), 0);
    assert!(slot.lock().unwrap().is_none());
}

#[tokio::test]
async fn a_user_denial_still_stops_on_the_first_refusal() {
    let (mw, handle, slot) = breaker();
    run_call(
        &mw,
        "deny-1",
        "shell",
        serde_json::json!({"command": "rm -rf build"}),
        Err("[policy-denied] The user declined this command"),
    )
    .await;
    assert_eq!(drain_pause_count(&handle), 1);
    let summary = slot.lock().unwrap().clone().unwrap();
    assert!(summary.contains("`policy`"), "{summary}");
}

// ── the timeout floor, by production tool name ──────────────────────────────

#[cfg(feature = "modules")]
fn search_spec(name: &str) -> tinysearch_bus::ToolSpec {
    tinysearch_bus::ToolSpec {
        name: name.to_string(),
        description: format!("{name} test spec"),
        parameters: serde_json::json!({"type": "object"}),
    }
}

/// The read tools as production registers them, so their own declarations
/// (permission level, policy, external effect) decide the timeout class.
#[cfg(feature = "modules")]
fn production_read_tools() -> Vec<Box<dyn tinytools::Tool>> {
    use crate::security::SecurityPolicy;
    let security = std::sync::Arc::new(SecurityPolicy::default());
    let tools: Vec<Box<dyn tinytools::Tool>> = vec![
        Box::new(crate::tools::implementations::network::web_fetch_tool(
            &crate::core::runtime::DispatchCtx::ambient(),
            security.clone(),
            vec!["example.com".into()],
            None,
            None,
        )),
        Box::new(tinytools_std::filesystem::FileReadTool::new(
            security.clone(),
        )),
        Box::new(tinytools_std::filesystem::ListFilesTool::new(security)),
    ];
    #[cfg(feature = "modules")]
    let tools = {
        let mut tools = tools;
        tools.push(Box::new(crate::desktop::control::tools::DesktopTool::new(
            std::sync::Arc::new(crate::config::Config::default()),
            crate::desktop::control::tools::DesktopToolKind::Windows,
        )));
        for name in ["web_search_tool", "web_answer_tool", "web_contents_tool"] {
            tools.push(Box::new(crate::search::tools::TinySearchTool::recorded(
                search_spec(name),
            )));
        }
        tools
    };
    tools
}

const PRODUCTION_READ_TOOLS: &[&str] = &[
    "web_fetch",
    "file_read",
    "list_files",
    #[cfg(feature = "modules")]
    "desktop_list_windows",
    #[cfg(feature = "modules")]
    "web_search_tool",
    #[cfg(feature = "modules")]
    "web_answer_tool",
    #[cfg(feature = "modules")]
    "web_contents_tool",
];

#[test]
#[cfg(feature = "modules")]
fn production_read_tools_time_out_as_transient() {
    let lookup = tool_sets_lookup(vec![std::sync::Arc::new(production_read_tools())]);
    for name in PRODUCTION_READ_TOOLS {
        let args = serde_json::json!({"path": "src", "url": "https://example.com", "query": "q"});
        let effect = call_effect(Some(&lookup), name, &args);
        assert_eq!(effect, CallEffect::ReadOnly, "{name}");
        assert_eq!(
            recovery_policy_with_effect(name, "timed out", false, effect),
            Some(("transient", 2)),
            "{name}"
        );
        // Without the registry (name only), the same answer.
        assert_eq!(
            recovery_policy(name, "timed out", false),
            Some(("transient", 2)),
            "{name} by name"
        );
    }
}

// ── a browser task that ended without finishing ─────────────────────────────

#[test]
fn a_failed_browser_task_gets_one_changed_attempt_not_a_credential_halt() {
    // The task's own prose (a page's 403, a planner's timeout) is the task's
    // report, not the tool call's verdict: nothing about the call is
    // uncertain, the task says what it did and what to change.
    for reason in [
        "Planning failed: the planner returned no steps",
        "the site answered 403 Forbidden on the login page",
        "the planner timed out choosing the next step",
    ] {
        let error = format!(
            "{} Browser task failed at step 1: {reason} Hint: narrow the goal",
            tinytools::status::TASK_FAILED_MARKER
        );
        assert_eq!(
            recovery_policy("browser", &error, false),
            Some(("task_failed", 1)),
            "{reason}"
        );
    }
    // A model gateway failing under the rescuer is transient.
    let error = format!(
        "{} Browser task failed at step 3: the rescuer gave up: rescue model returned HTTP 504",
        tinytools::status::TASK_FAILED_MARKER
    );
    assert_eq!(
        recovery_policy("browser", &error, false),
        Some(("transient", 2))
    );
}

#[test]
fn a_failed_task_is_classified_by_its_headline_not_its_attached_report() {
    // The tool error carries the task view as a report after the headline.
    // Words in that report (a page that said "authentication failed") must
    // not override the headline's gateway 504.
    let error = format!(
        "{} Browser task failed at step 3: the rescuer gave up: rescue model returned HTTP 504\n\n{}",
        tinytools::status::TASK_FAILED_MARKER,
        serde_json::json!({"summary": "the site said authentication failed on the login page"})
    );
    assert_eq!(
        recovery_policy("browser", &error, false),
        Some(("transient", 2))
    );
}

#[tokio::test]
async fn a_failed_browser_task_nudges_toward_its_hint_then_stops_on_a_repeat() {
    let (mw, handle, slot) = breaker();
    let error = format!(
        "{} Browser task failed at step 1: Planning failed Hint: name the exact button",
        tinytools::status::TASK_FAILED_MARKER
    );
    let args =
        serde_json::json!({"action": "task", "goal": "book it", "url": "https://shop.test/"});
    run_call(&mw, "task-1", "browser", args.clone(), Err(&error)).await;
    assert_eq!(drain_pause_count(&handle), 0);
    let nudges = drain_nudge_messages(&mw);
    assert_eq!(nudges.len(), 1, "{nudges:?}");
    assert!(nudges[0].contains("hint"), "{nudges:?}");
    run_call(&mw, "task-2", "browser", args, Err(&error)).await;
    assert_eq!(drain_pause_count(&handle), 1);
    let summary = slot.lock().unwrap().clone().unwrap();
    assert!(summary.contains("task_failed"), "{summary}");
}
