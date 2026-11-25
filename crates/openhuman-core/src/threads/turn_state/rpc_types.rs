//! RPC payloads for the `threads.turn_state_*` controllers, plus the host
//! conversion from a classified tool failure into its persisted form.
//!
//! The snapshot shapes and the store live in
//! `tinyagents_session::turn_state`; only the OpenHuman RPC envelopes and the
//! `ClassifiedFailure` projection stay host-side.

use serde::{Deserialize, Serialize};
use tinyagents_session::turn_state::types::{PersistedToolFailure, TurnState};

/// Request payload for `openhuman.threads_turn_state_get`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetTurnStateRequest {
    pub thread_id: String,
}

/// Response payload for `openhuman.threads_turn_state_get`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTurnStateResponse {
    /// `None` when no snapshot exists for the thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_state: Option<TurnState>,
}

/// Response payload for `openhuman.threads_turn_state_list` and
/// `openhuman.threads_turn_state_history`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTurnStatesResponse {
    pub turn_states: Vec<TurnState>,
    pub count: usize,
}

/// Request payload for `openhuman.threads_turn_state_get_turn` — a specific
/// turn of a thread, identified by its producing request id.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetTurnStateForRequestRequest {
    pub thread_id: String,
    pub request_id: String,
}

/// Request payload for `openhuman.threads_turn_state_clear`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearTurnStateRequest {
    pub thread_id: String,
}

/// Response payload for `openhuman.threads_turn_state_clear`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClearTurnStateResponse {
    pub cleared: bool,
}

/// Project a classified tool failure onto its persisted wire shape.
///
/// A free function rather than a `From` impl: both types are foreign to this
/// crate (`tinytools` and `tinyagents_session`).
pub(crate) fn persisted_tool_failure(
    f: &tinytools::status::ClassifiedFailure,
) -> PersistedToolFailure {
    {
        // Serialize the enums to their wire variant name so the persisted
        // `class`/`category` strings match exactly what the live socket emits
        // (`ClassifiedFailure` serializes each as its bare variant name).
        fn variant_name<T: Serialize>(v: &T) -> String {
            serde_json::to_value(v)
                .ok()
                .and_then(|j| j.as_str().map(str::to_string))
                .unwrap_or_default()
        }
        PersistedToolFailure {
            class: variant_name(&f.class),
            category: variant_name(&f.category),
            recoverable: f.recoverable,
            cause_plain: f.cause_plain.clone(),
            next_action: f.next_action.clone(),
        }
    }
}
