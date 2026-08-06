//! The `flows::` domain: saved automation workflows (tinyflows graphs) —
//! create/get/list/update/delete/enable/run, backed by SQLite. Mirrors
//! `src/openhuman/cron/`'s module shape.
//!
//! Business logic lives in [`ops`]; persistence in `store` (private, with a
//! handful of functions re-exported below for the capability seam's
//! [`crate::openhuman::flows::tinyflows::caps::FlowStateStore`]); the RPC/CLI
//! controller surface in `schemas` (private, re-exported below).
//!
//! [`medulla_bridge`] adapts this store onto the medulla harness protocol's
//! workflow plane, so a remote orchestrator can read these graphs and brief the
//! authoring copilot without any of that reaching back into `ops`.
//!
//! # Gate shape — leaf, not facade
//!
//! The whole family (this module plus [`tinyflows`] and [`rhai`]) is gated at
//! `pub mod flows;` in `src/openhuman/mod.rs` on `#[cfg(feature = "flows")]`,
//! and the submodules below inherit that gate. There is **no `stub.rs`**:
//! every symbol reached from outside is a *registration site* (`core::all`,
//! `core::jsonrpc`'s `FlowTriggerSubscriber`, `core::runtime::services`' boot
//! reconcile, the agent-tool `vec!` in `tools::ops`, the `workflow_builder` /
//! `flow_discovery` entries in `agent_registry`), and a registration site wants
//! *absence*, not a disabled-error stub — otherwise `flows.*` becomes a known
//! method that fails at runtime.
//!
//! The leaf gate holds only because no always-compiled domain has a real code
//! edge into this tree. `memory/tools.rs` and `memory/tools/flavour.rs` name
//! `flows::tinyflows` in **comments only**. If either ever becomes a real
//! `use`, this family must convert to the facade+stub shape (see `voice/`).

pub mod agents;
mod build_registry;
pub mod builder_tools;
pub mod bus;
pub mod discovery_tools;
mod draft_store;
pub mod medulla_bridge;
pub mod memory_tools;
mod n8n_import;
pub mod node_contracts;
pub mod ops;
/// The `.ragsh` language-workflow tool (formerly `openhuman::rhai_workflows`).
pub mod rhai;
mod run_registry;
mod schemas;
mod store;
/// The tinyflows engine seam (formerly `openhuman::tinyflows`).
pub mod tinyflows;
pub mod tools;
mod types;

pub use schemas::{
    all_controller_schemas as all_flows_controller_schemas,
    all_registered_controllers as all_flows_registered_controllers,
};
// `kv_get`/`kv_set` are re-exported (not just `pub(crate)`-visible within this
// domain's own module tree) because `tinyflows::caps::FlowStateStore`
// (`src/openhuman/flows/tinyflows/caps.rs`) lives in a sibling domain and needs
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
// `FLOW_MEMORY_NAMESPACE_PREFIX` / `flow_namespace` live in `memory_tools`
// (the domain logic sibling that owns the agent tools consuming them) and are
// re-exported here so every existing `flows::flow_namespace` /
// `flows::FLOW_MEMORY_NAMESPACE_PREFIX` call site (`bus.rs`, `ops.rs`, this
// module's own doc comments) keeps resolving unchanged — `mod.rs` stays
// export-focused only, per this repo's canonical module shape.
// `cross_flow_recall` is re-exported for the same reason: the tinyflows
// `memory` node's `OpenHumanMemory` adapter (`scope: "flows"` recall) must
// see byte-identical cross-flow results to `flow_memory_recall`'s own
// `scope: "flows"` arm, so both call the one implementation here rather than
// each walking `namespace_summaries` independently.
pub use memory_tools::{cross_flow_recall, flow_namespace, FLOW_MEMORY_NAMESPACE_PREFIX};
