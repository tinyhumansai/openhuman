//! JSON Schema for the thread and persisted turn records consumed by the UI.

use schemars::JsonSchema;

use crate::threads::rpc_models::{
    ConversationMessageRecord, ConversationMessagesResponse, ConversationThreadSummary,
    ConversationThreadsListResponse, DeleteConversationThreadResponse,
    PurgeConversationThreadsResponse,
};

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunLedgerGetWireResponse {
    pub run: Option<tinyagents_session::run_ledger::types::AgentRun>,
}

// Schema-only mirror of `tinyagents_harness::middleware::RunMode`. The pinned
// tinyagents (c3d7d32) derives `JsonSchema` on it without `rename_all`, so its
// schema names the variants `Build`/`Plan`, while the wire carries the
// `run_mode::mode_label` labels `build`/`plan`. Remove this once the pin
// includes tinyagents bafab14, which adds `#[serde(rename_all = "snake_case")]`.
/// A per-run mode gating side-effecting tools, toggled by the host without
/// restarting the run.
///
/// `Build` (the default) leaves tool exposure and execution unrestricted.
/// `Plan` hides every side-effecting tool from the model
/// ([`PlanModeMiddleware`]'s `before_model`, the same auditable
/// `AgentEvent::ToolsFiltered` mechanism [`ContextualToolSelectionMiddleware`]
/// uses) and denies it at execution (`before_tool`, the same
/// [`ToolPolicy`]/[`ToolSideEffects`] classification
/// [`ToolPolicyMiddleware::deny_side_effects`] enforces) — except for tools on
/// the middleware's allowlist, which stay available in either mode so a host
/// can keep read-only tools and a handful of plan-mode-specific tools (e.g.
/// `plan_exit`, `request_plan_review`, `todo`) reachable while planning.
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    /// Every registered tool may be exposed and executed. The default.
    Build,
    /// Only allowlisted and side-effect-free tools may be exposed or executed.
    Plan,
}

/// Root schema whose properties provide the UI-facing thread record aliases.
#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadWireTypes {
    pub thread: ConversationThreadSummary,
    pub message: ConversationMessageRecord,
    pub queued_chat_ack: super::rpc_models::QueuedChatAck,
    pub started_chat_ack: super::rpc_models::StartedChatAck,
    pub threads_list: ConversationThreadsListResponse,
    pub thread_messages: ConversationMessagesResponse,
    pub thread_delete: DeleteConversationThreadResponse,
    pub purge_result: PurgeConversationThreadsResponse,
    pub thread_search_hit: super::ops::ThreadSearchHit,
    pub thread_search: super::ops::ThreadSearchResponse,
    pub get_turn_state: super::turn_state::rpc_types::GetTurnStateResponse,
    pub list_turn_states: super::turn_state::rpc_types::ListTurnStatesResponse,
    pub clear_turn_state: super::turn_state::rpc_types::ClearTurnStateResponse,
    pub agent_run: tinyagents_session::run_ledger::types::AgentRun,
    pub run_event: tinyagents_session::run_ledger::types::RunEvent,
    pub run_telemetry: tinyagents_session::run_ledger::types::RunTelemetry,
    pub agent_run_list: tinyagents_session::run_ledger::types::AgentRunListResponse,
    pub agent_run_get: RunLedgerGetWireResponse,
    pub run_event_list: tinyagents_session::run_ledger::types::RunEventListResponse,
    pub turn_citation: crate::web_chat::TurnCitation,
    pub queue_mode: tinychannels_bus::web::QueueMode,
    pub artifact_kind: crate::artifacts::types::ArtifactKind,
    pub artifact_pending_payload: crate::artifacts::types::ArtifactPendingPayload,
    pub artifact_ready_payload: crate::artifacts::types::ArtifactReadyPayload,
    pub artifact_failed_payload: crate::artifacts::types::ArtifactFailedPayload,
    pub thread_goal: tinyagents_harness_tools::goals::ThreadGoal,
    pub thread_todo_item: tinyagents_harness_tools::todos::TodoItem,
    pub run_mode: RunMode,
}

/// Serialize the authoritative core/session record schemas for TypeScript codegen.
pub fn thread_wire_schema_json() -> Result<String, serde_json::Error> {
    let schema = schemars::generate::SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<ThreadWireTypes>();
    serde_json::to_string_pretty(&schema)
}

#[cfg(test)]
#[path = "wire_schema_tests.rs"]
mod tests;
