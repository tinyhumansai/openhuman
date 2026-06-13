//! Process-wide file state coordinator for cross-agent staleness detection.
//!
//! Parallel subagents and worker threads share a workspace. Without
//! coordination one worker can read a file, a sibling can edit it, and
//! the first worker can later write based on stale content. This module
//! tracks per-agent read stamps and per-path write stamps so that write
//! tools can detect the conflict and return a model-facing error
//! requiring the agent to re-read.
//!
//! Disable with `OPENHUMAN_FILE_STATE_GUARD=0` (or `false`).

mod agent_context;
mod ops;
mod types;

pub use agent_context::{current_file_state_agent_id, with_file_state_agent_id};
pub use ops::{
    acquire_path_lock, check_partial_read, check_stale_read, init_global, parent_stale_files,
    record_read, record_write, try_global,
};
pub use types::{FileStateCoordinator, ReadStamp};
