//! Background agent command center (issue #3373).
//!
//! A read-only product surface over the durable run ledger
//! (`session_db::run_ledger`): it lists recent background agent runs grouped by
//! a normalized status model (needs-input / working / completed / failed /
//! stopped) so users can see what is in flight, what is blocked on them, and
//! what finished. Live run state already persists to the ledger via the spawn
//! tools + `channels::providers::web::progress_bridge`; this module only
//! projects and groups it for display.
//!
//! Control verbs (stop / retry / continue) are intentionally out of scope here
//! and tracked as follow-up work.

mod ops;
mod schemas;
pub mod types;

pub use ops::{bucket_for, build_view, list_agent_work};
pub use schemas::{
    all_controller_schemas as all_command_center_controller_schemas,
    all_registered_controllers as all_command_center_registered_controllers,
};
pub use types::{AgentWorkBucket, AgentWorkRow, CommandCenterGroup, CommandCenterView};
