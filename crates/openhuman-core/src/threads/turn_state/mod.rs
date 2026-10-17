//! Restart-survivable snapshots of in-flight agent turns.
//!
//! The `tinyagents_session::port::TurnStates` port is the persistence layer
//! (reached through `agent::session_store::turn_states_for`): one snapshot per
//! turn in the storage backend, so a thread's turn state survives a restart;
//! `mark_all_interrupted` flags any non-terminal snapshot left over from an
//! unclean shutdown at cold boot.
//! `tinyagents_session::turn_state::TurnStateMirror` is the writer — it is driven by
//! [`mirror::ObserveProgress`], which translates
//! [`crate::agent::progress::AgentProgress`] events into [`types::TurnState`]
//! mutations and flushes to the store at iteration / tool boundaries (not on
//! every streaming delta), keeping `Completed` snapshots so the UI can replay
//! a finished turn.
//!
//! [`types`] holds the wire/storage shapes; they are camelCase to mirror
//! `app/src/store/chatRuntimeSlice.ts`. `TurnStateMirror` is driven by the
//! web-channel progress bridge (`web_chat::progress_bridge`), and snapshots
//! are read back through the `threads` RPC surface (`turn_state_get` /
//! `turn_state_list` / `turn_state_history` / `turn_state_get_turn` /
//! `turn_state_clear`). See `../README.md` for the full `threads` module
//! picture.

pub mod mirror;
pub mod rpc_types;

pub use rpc_types::{
    ClearTurnStateRequest, ClearTurnStateResponse, GetTurnStateForRequestRequest,
    GetTurnStateRequest, GetTurnStateResponse, ListTurnStatesResponse,
};
