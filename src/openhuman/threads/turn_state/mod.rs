//! Persistent per-thread snapshots of in-flight agent turns.
//!
//! See the rustdoc on [`types::TurnState`] for the snapshot shape and
//! [`store::TurnStateStore`] for the on-disk layout. The web-channel
//! progress consumer writes to this store at iteration / tool
//! boundaries; the [`crate::openhuman::threads`] RPC surface lets the
//! UI rehydrate its `chatRuntimeSlice` after a navigation or restart.

pub mod mirror;
pub mod store;
pub mod types;

pub use mirror::TurnStateMirror;

pub use store::TurnStateStore;
pub use types::{
    ClearTurnStateRequest, ClearTurnStateResponse, GetTurnStateRequest, GetTurnStateResponse,
    ListTurnStatesResponse, SubagentActivity, SubagentToolCall, ToolTimelineEntry,
    ToolTimelineStatus, TurnLifecycle, TurnPhase, TurnState,
};
