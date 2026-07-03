//! The `flows::` domain: saved automation workflows (tinyflows graphs) —
//! create/get/list/update/delete/enable/run, backed by SQLite. Mirrors
//! `src/openhuman/cron/`'s module shape.
//!
//! Business logic lives in [`ops`]; persistence in `store` (private, with a
//! handful of functions re-exported below for the capability seam's
//! [`crate::openhuman::tinyflows::caps::FlowStateStore`]); the RPC/CLI
//! controller surface in `schemas` (private, re-exported below).

pub mod bus;
pub mod ops;
mod schemas;
mod store;
mod types;

pub use schemas::{
    all_controller_schemas as all_flows_controller_schemas,
    all_registered_controllers as all_flows_registered_controllers,
};
// `kv_get`/`kv_set` are re-exported (not just `pub(crate)`-visible within this
// domain's own module tree) because `tinyflows::caps::FlowStateStore`
// (`src/openhuman/tinyflows/caps.rs`) lives in a sibling domain and needs
// them to implement `tinyflows::caps::StateStore` without duplicating the
// `flow_state` table's persistence logic.
pub use store::{kv_get, kv_set};
pub use types::Flow;
