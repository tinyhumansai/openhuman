//! Regression tests for the tool-failure breaker ending turns too early
//! (Langfuse, Oct 2026: 3.4% of orchestrator turns ended with "I stopped this
//! turn early" and the user re-asked).
//!
//! - a timeout on a read-only call is transient, not an uncertain side effect;
//! - `shell` "command not found" gets one correction attempt, with a nudge
//!   naming the host OS and shell;
//! - `use_skill` failures are counted per inner skill + tool.

use super::super::call_effect::{
    call_effect, dispatch_target, CallEffect, ToolEffectFacts, ToolFactsLookup,
};
use super::super::failure_policy::recovery_policy_with_effect;
use super::super::repeated_failure::{failure_scope, recovery_policy};
use super::*;

/// The harness's own wording for a tool that outlived its budget
/// (`tinyagents` `agent_loop/tools.rs`).
fn harness_timeout(tool: &str) -> String {
    format!("tool `{tool}` timed out after 30000 ms")
}

async fn run_call(
    mw: &RepeatedToolFailureMiddleware,
    id: &str,
    tool: &str,
    args: serde_json::Value,
    error: &str,
) {
    let mut call = TaToolCall::new(id, tool, args);
    mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
    let mut result = failing_result(tool, error);
    mw.after_tool(&mut ctx(), &(), &invocation(id, tool), &mut result)
        .await
        .unwrap();
}

// ── 1. timeouts are classified by side effect, not by a name allowlist ──────

#[test]
fn a_read_only_timeout_is_transient() {
    // `composio_list_tools` was not on the old five-name allowlist, so its
    // timeout was an `uncertain_side_effect` with zero retries.
    assert_eq!(
        recovery_policy(
            "composio_list_tools",
            &harness_timeout("composio_list_tools"),
            false
        ),
        Some(("transient", 2))
    );
    assert_eq!(
        recovery_policy_with_effect(
            "frobnicate",
            &harness_timeout("frobnicate"),
            false,
            CallEffect::ReadOnly
        ),
        Some(("transient", 2))
    );
    // The old allowlist still holds whatever the metadata says.
    assert_eq!(
        recovery_policy_with_effect("web_fetch", "timed out", false, CallEffect::Unknown),
        Some(("transient", 2))
    );
}

#[test]
fn a_side_effecting_timeout_stays_uncertain() {
    assert_eq!(
        recovery_policy("gmail_send", "timed out", false),
        Some(("uncertain_side_effect", 0))
    );
    assert_eq!(
        recovery_policy_with_effect(
            "list_and_send",
            "timed out",
            false,
            CallEffect::SideEffecting
        ),
        Some(("uncertain_side_effect", 0))
    );
    // A tool nobody can classify is treated as side-effecting.
    assert_eq!(
        recovery_policy_with_effect("frobnicate", "timed out", false, CallEffect::Unknown),
        Some(("uncertain_side_effect", 0))
    );
}

#[test]
fn a_timeout_before_the_tool_ran_is_transient() {
    // Python runtime resolution timing out: nothing ran, so nothing to
    // reconcile, even for an execute-class tool.
    assert_eq!(
        recovery_policy_with_effect(
            "shell",
            "Python runtime unavailable: runtime resolution timed out after 60s",
            false,
            CallEffect::SideEffecting
        ),
        Some(("transient", 2))
    );
    // A schema rejection that merely names a `timeout_secs` field is a wrong
    // call, not a timeout.
    assert_eq!(
        recovery_policy_with_effect(
            "shell",
            "invalid arguments for tool `shell`: timeout_secs must be an integer",
            false,
            CallEffect::SideEffecting
        ),
        Some((
            "invalid_arguments",
            super::super::failure_policy::ARGUMENT_SCHEMA_RECOVERY
        ))
    );
}

#[test]
fn the_effect_of_a_dispatcher_is_the_effect_of_its_target() {
    let list = serde_json::json!({"skill": "composio", "tool": "composio_list_tools", "args": {}});
    assert_eq!(call_effect(None, "use_skill", &list), CallEffect::ReadOnly);
    let send = serde_json::json!({"skill": "gmail", "tool": "gmail_send_email", "args": {}});
    assert_eq!(
        call_effect(None, "use_skill", &send),
        CallEffect::SideEffecting
    );
    let fetch = serde_json::json!({"tool": "GMAIL_FETCH_EMAILS", "arguments": {}});
    assert_eq!(
        call_effect(None, "composio_execute", &fetch),
        CallEffect::ReadOnly
    );
    let target = dispatch_target("use_skill", &list).expect("use_skill names a target");
    assert_eq!(target.skill, Some("composio"));
    assert_eq!(target.tool, "composio_list_tools");
}

#[test]
fn declared_metadata_outranks_the_tool_name() {
    let lookup: ToolFactsLookup = std::sync::Arc::new(|name: &str, _args: &serde_json::Value| {
        match name {
            // Declared read-only, though its name says nothing.
            "frobnicate" => Some(ToolEffectFacts {
                classified: Some(CallEffect::ReadOnly),
                external: false,
                elevated: false,
            }),
            // A "list" that reaches outside the machine.
            "list_and_notify" => Some(ToolEffectFacts {
                classified: None,
                external: true,
                elevated: false,
            }),
            _ => None,
        }
    });
    let args = serde_json::json!({});
    assert_eq!(
        call_effect(Some(&lookup), "frobnicate", &args),
        CallEffect::ReadOnly
    );
    assert_eq!(
        call_effect(Some(&lookup), "list_and_notify", &args),
        CallEffect::SideEffecting
    );
}

/// The production shape: `use_skill` → `composio_list_tools` timing out once
/// halted the turn.
#[tokio::test]
async fn a_read_only_use_skill_timeout_does_not_end_the_turn() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let args = serde_json::json!({"skill": "composio", "tool": "composio_list_tools", "args": {}});
    run_call(
        &mw,
        "skill-1",
        "use_skill",
        args.clone(),
        &harness_timeout("use_skill"),
    )
    .await;
    run_call(
        &mw,
        "skill-2",
        "use_skill",
        args.clone(),
        &harness_timeout("use_skill"),
    )
    .await;
    assert_eq!(drain_pause_count(&handle), 0, "two retries are allowed");
    assert!(slot.lock().unwrap().is_none());
    run_call(
        &mw,
        "skill-3",
        "use_skill",
        args,
        &harness_timeout("use_skill"),
    )
    .await;
    assert_eq!(drain_pause_count(&handle), 1, "the budget is still bounded");
    let summary = slot.lock().unwrap().clone().unwrap();
    assert!(summary.contains("transient"), "{summary}");
}

#[tokio::test]
async fn a_side_effecting_use_skill_timeout_still_stops_at_once() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    run_call(
        &mw,
        "send-1",
        "use_skill",
        serde_json::json!({"skill": "gmail", "tool": "gmail_send_email", "args": {}}),
        &harness_timeout("use_skill"),
    )
    .await;
    assert_eq!(drain_pause_count(&handle), 1);
    let summary = slot.lock().unwrap().clone().unwrap();
    assert!(summary.contains("uncertain_side_effect"), "{summary}");
}

// ── 2. `shell` "command not found" gets one correction ──────────────────────

#[test]
fn a_missing_shell_command_gets_one_correction() {
    assert_eq!(
        recovery_policy("shell", "bash: jq: command not found", false),
        Some(("missing_app", 1))
    );
    // Other tools keep the zero-retry `unsupported` verdict.
    assert_eq!(
        recovery_policy("shell", "executable not found", false),
        Some(("unsupported", 0))
    );
}

#[tokio::test]
async fn a_missing_shell_command_nudges_with_the_host_os_then_stops() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    run_call(
        &mw,
        "sh-1",
        "shell",
        serde_json::json!({"command": "ls -la | jq ."}),
        "Sandbox execution failed: jq: command not found",
    )
    .await;
    assert_eq!(drain_pause_count(&handle), 0, "first miss must not halt");
    let nudges = mw.take_pending_nudges();
    assert_eq!(nudges.len(), 1, "{nudges:?}");
    assert!(nudges[0].contains(std::env::consts::OS), "{nudges:?}");
    assert!(nudges[0].contains("not available"), "{nudges:?}");
    assert!(
        !nudges[0].contains("desktop target"),
        "a shell miss is not a desktop window: {nudges:?}"
    );

    run_call(
        &mw,
        "sh-2",
        "shell",
        serde_json::json!({"command": "jq . file.json"}),
        "Sandbox execution failed: jq: command not found",
    )
    .await;
    assert_eq!(drain_pause_count(&handle), 1, "the second miss halts");
}

// ── 3. `use_skill` failures are counted per inner skill + tool ──────────────

#[test]
fn the_failure_scope_of_use_skill_names_its_inner_skill_and_tool() {
    let list = failure_scope(
        "use_skill",
        &serde_json::json!({"skill": "composio", "tool": "composio_list_tools", "args": {}}),
    );
    let exec = failure_scope(
        "use_skill",
        &serde_json::json!({
            "skill": "composio",
            "tool": "composio_execute",
            "args": {"tool": "GMAIL_FETCH_EMAILS", "account_id": "acct-1"}
        }),
    );
    assert_ne!(list, exec);
    assert!(list.contains("skill=composio"), "{list}");
    assert!(list.contains("tool=composio_list_tools"), "{list}");
    // The inner call's resource identity scopes it too.
    assert!(exec.contains("account_id=acct-1"), "{exec}");
    // Not a dispatcher: unchanged.
    assert_eq!(
        failure_scope("web_search", &serde_json::json!({"query": "x"})),
        "web_search"
    );
}

#[tokio::test]
async fn failures_of_different_use_skill_tools_do_not_share_a_budget() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 8, slot.clone());
    for (id, inner) in [("a", "tool_alpha"), ("b", "tool_beta"), ("c", "tool_gamma")] {
        run_call(
            &mw,
            id,
            "use_skill",
            serde_json::json!({"skill": "pack", "tool": inner, "args": {}}),
            r#"{"status_code": 503}"#,
        )
        .await;
    }
    assert_eq!(
        drain_pause_count(&handle),
        0,
        "three different sub-tools failing once each is not one blocker"
    );
    assert!(slot.lock().unwrap().is_none());

    // The same sub-tool failing past its budget still stops.
    for id in ["a2", "a3"] {
        run_call(
            &mw,
            id,
            "use_skill",
            serde_json::json!({"skill": "pack", "tool": "tool_alpha", "args": {}}),
            r#"{"status_code": 503}"#,
        )
        .await;
    }
    assert_eq!(drain_pause_count(&handle), 1);
    let summary = slot.lock().unwrap().clone().unwrap();
    assert!(summary.contains("tool_alpha"), "{summary}");
}

// ── web search timeouts (Langfuse, Oct 2026) ────────────────────────────────
//
// Production turns stopped with "failure class uncertain_side_effect still
// blocks operation web_answer_tool … The action may already have happened":
// a grounded answer outlived the 30 s bus timeout, and because the search
// tools declared no policy and `web_answer_tool` carries no reading verb, the
// timeout read as a possible side effect with zero retries.

#[cfg(feature = "modules")]
fn search_spec(name: &str) -> tinysearch_bus::ToolSpec {
    tinysearch_bus::ToolSpec {
        name: name.to_string(),
        description: format!("{name} test spec"),
        parameters: serde_json::json!({"type": "object"}),
    }
}

#[cfg(feature = "modules")]
#[test]
fn a_web_search_tool_timeout_is_transient_not_an_uncertain_action() {
    let tools: Vec<Box<dyn tinytools::Tool>> =
        ["web_answer_tool", "web_search_tool", "web_contents_tool"]
            .into_iter()
            .map(|name| {
                Box::new(crate::search::tools::TinySearchTool::recorded(search_spec(
                    name,
                ))) as Box<dyn tinytools::Tool>
            })
            .collect();
    let lookup = super::super::call_effect::tool_sets_lookup(vec![std::sync::Arc::new(tools)]);
    for name in ["web_answer_tool", "web_search_tool", "web_contents_tool"] {
        let args = serde_json::json!({"query": "what changed in the release"});
        let effect = call_effect(Some(&lookup), name, &args);
        assert_eq!(effect, CallEffect::ReadOnly, "{name}");
        assert_eq!(
            recovery_policy_with_effect(
                name,
                "search ExecuteTool failed: bus call timed out after 30s",
                false,
                effect
            ),
            Some(("transient", 2)),
            "{name}"
        );
    }
}
