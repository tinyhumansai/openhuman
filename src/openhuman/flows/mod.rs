//! The `flows::` domain: saved automation workflows (tinyflows graphs) —
//! create/get/list/update/delete/enable/run, backed by SQLite. Mirrors
//! `src/openhuman/cron/`'s module shape.
//!
//! Business logic lives in [`ops`]; persistence in `store` (private, with a
//! handful of functions re-exported below for the capability seam's
//! [`crate::openhuman::tinyflows::caps::FlowStateStore`]); the RPC/CLI
//! controller surface in `schemas` (private, re-exported below).

pub mod agents;
pub mod builder_tools;
pub mod bus;
pub mod discovery_tools;
mod draft_store;
mod n8n_import;
pub mod node_contracts;
pub mod ops;
mod run_registry;
mod schemas;
mod store;
pub mod tools;
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
// `upsert_flow_run_step` is likewise re-exported for the tinyflows seam: the
// live run observer (`tinyflows::observability::FlowRunObserver`, issue G2)
// lives in the sibling `tinyflows` domain and persists each finished step onto
// the `flow_runs` row through this function as the run executes.
pub use node_contracts::{
    all_node_kind_contracts, node_kind_contract, render_node_kinds_line, ConfigField,
    NodeKindContract, PortSpec, NODE_KINDS,
};
pub use store::{kv_get, kv_set, upsert_flow_run_step};
pub use types::{
    DraftOrigin, Flow, FlowConnection, FlowDraft, FlowImport, FlowRevision, FlowRun, FlowRunStep,
    FlowRunTrigger, FlowSuggestion, FlowValidation, FlowValidationError, SuggestionStatus,
};
