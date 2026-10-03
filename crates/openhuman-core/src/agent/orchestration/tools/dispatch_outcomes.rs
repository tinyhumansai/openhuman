//! Pure result mapping and framing for synchronous delegation dispatch.

use super::DispatchMode;
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
    ToolResult::success(super::super::awaiting_user::awaiting_user_envelope(
        &outcome.task_id,
        &outcome.agent_id,
        None,
        question,
        checkpointed,
    ))
}

pub(super) fn format_subagent_failure(tool_name: &str, message: &str) -> String {
    format!("{tool_name} failed and did not complete — no work was performed and no results were produced. Do NOT treat this as success or fabricate an output; report the failure to the user. Error: {message}")
}

pub(crate) fn is_unexecuted_tool_call_stub(output: &str) -> bool {
    use super::tool_call_text::{contains_tool_call_payload, strip_tool_calls_from_response};
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

const INLINE_RESULT_NOTE: &str = "\n\n[INLINE_RESULT] This delegation ran inline and is complete as returned — there is no sub-agent worker for it. Do NOT call wait_subagent, list_subagents or continue_subagent for this delegation.";
const NO_WORKER_NOTE: &str = "\n\n[INLINE_RESULT] This delegation ran inline and registered no sub-agent worker, so there is nothing to collect: do NOT call wait_subagent, list_subagents or continue_subagent for it. Re-delegate with a corrected prompt instead.";

pub(crate) fn with_inline_result_note(output: String, mode: DispatchMode) -> String {
    if mode == DispatchMode::Blocking {
        format!("{output}{INLINE_RESULT_NOTE}")
    } else {
        output
    }
}

pub(crate) fn incomplete_envelope(
    tool_name: &str,
    reason: &str,
    output: &str,
    mode: DispatchMode,
) -> String {
    let envelope = format!("[SUBAGENT_INCOMPLETE] the {tool_name} sub-agent {reason} and did not finish. Below is partial progress only — do NOT report it as done or re-run the identical delegation unchanged.\n\nPartial progress:\n{output}");
    if mode == DispatchMode::Blocking {
        format!("{envelope}{NO_WORKER_NOTE}")
    } else {
        envelope
    }
}
