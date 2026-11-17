//! Small typed read model for the fields the terminal transcript displays.
//!
//! The wire contract remains TinyChannels' tagged `WebChannelEvent`; this is a
//! consumer projection rather than another wire DTO.

use openhuman_rpc::embed::chat_surface::{SubagentProgressDetail, WebChannelEvent};
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub(crate) struct SubagentView {
    pub task_id: Option<String>,
    pub display_name: Option<String>,
    pub elapsed_ms: Option<u64>,
    pub output: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WebEventView {
    pub event: String,
    pub client_id: String,
    pub thread_id: String,
    pub request_id: String,
    pub seq: Option<u64>,
    pub delta: Option<String>,
    pub full_response: Option<String>,
    pub message: Option<String>,
    pub error_retryable: Option<bool>,
    pub error_retry_after_ms: Option<u64>,
    pub error_type: Option<String>,
    pub output: Option<String>,
    pub skill_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub success: Option<bool>,
    pub args: Option<Value>,
    pub elapsed_ms: Option<u64>,
    pub tool_display_label: Option<String>,
    pub subagent: Option<SubagentView>,
    pub failure: Option<Value>,
}

impl WebEventView {
    pub(crate) fn from_event(event: &WebChannelEvent) -> Self {
        let context = event.context();
        let mut view = Self {
            event: event.event_name().to_owned(),
            client_id: context.client_id.clone(),
            thread_id: context.thread_id.clone(),
            request_id: context.request_id.clone(),
            seq: context.seq,
            ..Self::default()
        };
        use WebChannelEvent as E;
        let subagent_view = |detail: &SubagentProgressDetail| SubagentView {
            task_id: detail.task_id.clone(),
            display_name: detail.display_name.clone(),
            elapsed_ms: detail.elapsed_ms,
            output: detail.output.clone(),
        };
        match event {
            E::ChatDone { full_response, .. }
            | E::ChatInterim { full_response, .. }
            | E::ChatMessage { full_response, .. }
            | E::ChatSegment { full_response, .. }
            | E::ProactiveMessage { full_response, .. }
            | E::VoiceSpeak { full_response, .. }
            | E::ChannelOutbound { full_response, .. } => {
                view.full_response.clone_from(full_response);
            }
            E::ChatError {
                message,
                error_type,
                error_retryable,
                error_retry_after_ms,
                ..
            } => {
                view.message.clone_from(message);
                view.error_type.clone_from(error_type);
                view.error_retryable.clone_from(error_retryable);
                view.error_retry_after_ms.clone_from(error_retry_after_ms);
            }
            E::ChatCancelled { cancel_reason, .. } => {
                view.message.clone_from(cancel_reason);
            }
            E::TextDelta { delta, .. } | E::ThinkingDelta { delta, .. } => {
                view.delta.clone_from(delta);
            }
            E::SubagentTextDelta {
                delta,
                skill_id,
                subagent,
                ..
            }
            | E::SubagentThinkingDelta {
                delta,
                skill_id,
                subagent,
                ..
            } => {
                view.delta.clone_from(delta);
                view.skill_id.clone_from(skill_id);
                view.subagent = subagent.as_ref().map(subagent_view);
            }
            E::IterationStart { message, .. } | E::RunModeChanged { message, .. } => {
                view.message.clone_from(message);
            }
            E::UserError { error_type, .. } => view.error_type.clone_from(error_type),
            E::ToolCall {
                tool_name,
                skill_id,
                args,
                tool_call_id,
                tool_display_label,
                ..
            } => {
                view.tool_name.clone_from(tool_name);
                view.skill_id.clone_from(skill_id);
                view.args.clone_from(args);
                view.tool_call_id.clone_from(tool_call_id);
                view.tool_display_label.clone_from(tool_display_label);
            }
            E::SubagentToolCall {
                tool_name,
                skill_id,
                args,
                tool_call_id,
                tool_display_label,
                subagent,
                ..
            } => {
                view.tool_name.clone_from(tool_name);
                view.skill_id.clone_from(skill_id);
                view.args.clone_from(args);
                view.tool_call_id.clone_from(tool_call_id);
                view.tool_display_label.clone_from(tool_display_label);
                view.subagent = subagent.as_ref().map(subagent_view);
            }
            E::ApprovalRequest {
                tool_name,
                args,
                tool_call_id,
                message,
                ..
            }
            | E::PlanReviewRequest {
                tool_name,
                args,
                tool_call_id,
                message,
                ..
            } => {
                view.tool_name.clone_from(tool_name);
                view.args.clone_from(args);
                view.tool_call_id.clone_from(tool_call_id);
                view.message.clone_from(message);
            }
            E::ApprovalDecided {
                tool_name,
                message,
                tool_call_id,
                ..
            }
            | E::PlanReviewDecided {
                tool_name,
                message,
                tool_call_id,
                ..
            } => {
                view.tool_name.clone_from(tool_name);
                view.message.clone_from(message);
                view.tool_call_id.clone_from(tool_call_id);
            }
            E::ToolResult {
                tool_name,
                skill_id,
                output,
                args,
                success,
                tool_call_id,
                failure,
                elapsed_ms,
                tool_display_label,
                ..
            } => {
                view.tool_name.clone_from(tool_name);
                view.skill_id.clone_from(skill_id);
                view.output.clone_from(output);
                view.args.clone_from(args);
                view.success.clone_from(success);
                view.tool_call_id.clone_from(tool_call_id);
                view.failure.clone_from(failure);
                view.elapsed_ms.clone_from(elapsed_ms);
                view.tool_display_label.clone_from(tool_display_label);
            }
            E::SubagentToolResult {
                tool_name,
                skill_id,
                output,
                args,
                success,
                tool_call_id,
                failure,
                elapsed_ms,
                tool_display_label,
                subagent,
                ..
            } => {
                view.tool_name.clone_from(tool_name);
                view.skill_id.clone_from(skill_id);
                view.output.clone_from(output);
                view.args.clone_from(args);
                view.success.clone_from(success);
                view.tool_call_id.clone_from(tool_call_id);
                view.failure.clone_from(failure);
                view.elapsed_ms.clone_from(elapsed_ms);
                view.tool_display_label.clone_from(tool_display_label);
                view.subagent = subagent.as_ref().map(subagent_view);
            }
            E::ToolArgsDelta {
                delta,
                tool_name,
                skill_id,
                tool_call_id,
                ..
            } => {
                view.delta.clone_from(delta);
                view.tool_name.clone_from(tool_name);
                view.skill_id.clone_from(skill_id);
                view.tool_call_id.clone_from(tool_call_id);
            }
            E::ToolProgress {
                tool_call_id,
                message,
                ..
            } => {
                view.tool_call_id.clone_from(tool_call_id);
                view.message.clone_from(message);
            }
            E::SubagentSpawned {
                tool_name,
                skill_id,
                subagent,
                message,
                ..
            }
            | E::SubagentIterationStart {
                tool_name,
                skill_id,
                subagent,
                message,
                ..
            } => {
                view.tool_name.clone_from(tool_name);
                view.skill_id.clone_from(skill_id);
                view.message.clone_from(message);
                view.subagent = subagent.as_ref().map(subagent_view);
            }
            E::SubagentCompleted {
                tool_name,
                skill_id,
                subagent,
                message,
                success,
                ..
            }
            | E::SubagentFailed {
                tool_name,
                skill_id,
                subagent,
                message,
                success,
                ..
            }
            | E::SubagentAwaitingUser {
                tool_name,
                skill_id,
                subagent,
                message,
                success,
                ..
            } => {
                view.tool_name.clone_from(tool_name);
                view.skill_id.clone_from(skill_id);
                view.message.clone_from(message);
                view.success.clone_from(success);
                view.subagent = subagent.as_ref().map(subagent_view);
            }
            E::ArtifactReady {
                tool_call_id, args, ..
            }
            | E::ArtifactFailed {
                tool_call_id, args, ..
            }
            | E::ArtifactPending {
                tool_call_id, args, ..
            } => {
                view.tool_call_id.clone_from(tool_call_id);
                view.args.clone_from(args);
            }
            // Other owner variants carry no fields consumed by the terminal transcript.
            _ => {}
        }
        view
    }
}

#[cfg(test)]
#[path = "event_view_tests.rs"]
mod tests;
