//! RPC accessors over persisted in-flight turn snapshots.

use super::support::{counts, envelope, workspace_dir};
use crate::core::Outcome;
use crate::threads::turn_state::{
    ClearTurnStateRequest, ClearTurnStateResponse, GetTurnStateForRequestRequest,
    GetTurnStateRequest, GetTurnStateResponse, ListTurnStatesResponse,
};
use crate::threads::{ApiEnvelope, EmptyRequest};
use tinyagents_session::turn_state;

/// Returns the persisted in-flight turn snapshot for a thread, if any.
pub async fn turn_state_get(
    request: GetTurnStateRequest,
) -> Result<Outcome<ApiEnvelope<GetTurnStateResponse>>, String> {
    let dir = workspace_dir().await?;
    let turn_state = turn_state::store::get(dir, &request.thread_id)?;
    let present = turn_state.is_some();
    Ok(envelope(
        GetTurnStateResponse { turn_state },
        Some(counts([("present", usize::from(present))])),
        None,
    ))
}

/// Lists every persisted turn snapshot — used by the UI on cold boot to
/// surface interrupted turns from a previous process.
pub async fn turn_state_list(
    _request: EmptyRequest,
) -> Result<Outcome<ApiEnvelope<ListTurnStatesResponse>>, String> {
    let dir = workspace_dir().await?;
    let turn_states = turn_state::store::list(dir)?;
    let count = turn_states.len();
    Ok(envelope(
        ListTurnStatesResponse { turn_states, count },
        Some(counts([("num_turn_states", count)])),
        None,
    ))
}

/// Lists every persisted turn snapshot for one thread, newest first — the
/// per-turn history that lets the UI render each answer's own process trail.
pub async fn turn_state_history(
    request: GetTurnStateRequest,
) -> Result<Outcome<ApiEnvelope<ListTurnStatesResponse>>, String> {
    let dir = workspace_dir().await?;
    let turn_states = turn_state::store::list_thread(dir, &request.thread_id)?;
    let count = turn_states.len();
    Ok(envelope(
        ListTurnStatesResponse { turn_states, count },
        Some(counts([("num_turn_states", count)])),
        None,
    ))
}

/// Returns one specific turn of a thread by its producing request id — used by
/// the UI to lazily load a past turn's full timeline when its insights block is
/// first expanded.
pub async fn turn_state_get_turn(
    request: GetTurnStateForRequestRequest,
) -> Result<Outcome<ApiEnvelope<GetTurnStateResponse>>, String> {
    let dir = workspace_dir().await?;
    let turn_state = turn_state::store::get_turn(dir, &request.thread_id, &request.request_id)?;
    let present = turn_state.is_some();
    Ok(envelope(
        GetTurnStateResponse { turn_state },
        Some(counts([("present", usize::from(present))])),
        None,
    ))
}

/// Clears the persisted turn snapshot for a thread (e.g. after the user
/// dismisses an "interrupted" banner).
pub async fn turn_state_clear(
    request: ClearTurnStateRequest,
) -> Result<Outcome<ApiEnvelope<ClearTurnStateResponse>>, String> {
    let dir = workspace_dir().await?;
    let cleared = turn_state::store::delete(dir, &request.thread_id)?;
    Ok(envelope(ClearTurnStateResponse { cleared }, None, None))
}
