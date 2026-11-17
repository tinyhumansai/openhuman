//! Pure result mapping and framing for synchronous delegation dispatch.

use tinytools::ToolResult;

pub(super) fn awaiting_outcome_to_tool_result(
    outcome: &crate::agent::subagent_host::SubagentRunOutcome,
    question: &str,
    checkpointed: bool,
) -> ToolResult {
    if !checkpointed {
        let question_json = serde_json::to_string(question)
            .unwrap_or_else(|_| "\"<unserializable question>\"".into());
        return ToolResult::error(format!(
            "The sub-agent `{}` paused to ask a question, but its state could not be saved and this delegation has no durable session to fall back on, so it cannot be resumed. Its progress is lost. Tell the user what it was asking — {} — and that the delegation has to be started again; do NOT call continue_subagent with task_id `{}`, there is nothing for it to resume.",
            outcome.agent_id, question_json, outcome.task_id
        ));
    }
    ToolResult::success(super::awaiting_user::awaiting_user_envelope(
        &outcome.task_id,
        &outcome.agent_id,
        None,
        question,
        checkpointed,
    ))
}

pub(super) fn format_subagent_failure(tool_name: &str, message: &str) -> String {
    format!(
        "{tool_name} failed and did not complete — no work was performed and no results were produced. Do NOT treat this as success or fabricate an output; report the failure to the user. Error: {message}"
    )
}

pub(crate) fn is_unexecuted_tool_call_stub(output: &str) -> bool {
    use tinytools_agent::dialect::{contains_tool_call_payload, strip_tool_calls_from_response};
    if !contains_tool_call_payload(output) {
        return false;
    }
    if let Ok(serde_json::Value::Object(map)) =
        serde_json::from_str::<serde_json::Value>(output.trim())
    {
        if map.contains_key("tool_calls") {
            return true;
        }
    }
    !strip_tool_calls_from_response(output)
        .chars()
        .any(char::is_alphanumeric)
}

/// How a sub-agent handed back as `Incomplete { reason }` was stopped
/// (iteration cap or failure breaker), for its `SubagentCompleted` event.
pub(crate) fn incomplete_stop(
    reason: &str,
    failure_cause: Option<tinyagents_harness::no_progress::ClassifiedStopCause>,
) -> Option<tinyagents_harness::turn_stop::TurnStop> {
    Some(
        crate::agent::session_host::stop_policy::stop_from_incomplete_with_cause(
            reason,
            failure_cause,
        ),
    )
}

/// The parent's tool result for a sub-agent the harness stopped early: a
/// **failure** carrying `envelope` (the actionable "did not finish" framing
/// plus the stop summary and partial progress), so the parent's failure
/// policy and its trace see the stop. It used to be a success the parent
/// could narrate as done.
pub(crate) fn stopped_subagent_result(
    tool_name: &str,
    agent_id: &str,
    task_id: &str,
    reason: &str,
    failure_cause: Option<tinyagents_harness::no_progress::ClassifiedStopCause>,
    envelope: String,
) -> ToolResult {
    let stop = incomplete_stop(reason, failure_cause).expect("incomplete stop is present");
    log::debug!(
        "[subagent] {tool_name} child stopped early agent_id={agent_id} task_id={task_id} \
         {}; returning a failed tool result",
        stop.status_message()
    );
    ToolResult::error(envelope)
}
