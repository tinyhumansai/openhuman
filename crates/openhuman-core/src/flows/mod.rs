//! The `flows::` domain: saved automation workflows (tinyflows graphs) —
//! create/get/list/update/delete/enable/run, backed by SQLite. Mirrors
//! `crates/openhuman-core/src/cron/`'s module shape.
//!
//! Business logic lives in [`ops`]; persistence in `store` (private, with a
//! handful of functions re-exported below for the capability seam's
//! `tinyflows_sqlite::flows::SqliteStateStore`); the RPC/CLI
//! controller surface in `schemas` (private, re-exported below).
//!
//! # Gate shape — leaf, not facade
//!
//! The whole family (this module plus [`tinyflows`]) is gated at
//! `pub mod flows;` in `crates/openhuman-core/src/lib.rs` on `#[cfg(feature = "flows")]`,
//! and the submodules below inherit that gate. There is **no `stub.rs`**:
//! every symbol reached from outside is a *registration site* (`core::all`,
//! `core::runtime::subscribers`' `FlowTriggerSubscriber`, `core::runtime::services`' boot
//! reconcile, the agent-tool `vec!` in `tools::ops`, the `workflow_builder` /
//! `flow_discovery` entries in `agent::registry`'s `BUILTINS`), and a registration site wants
//! *absence*, not a disabled-error stub — otherwise `flows.*` becomes a known
//! method that fails at runtime.
//!
//! The leaf gate holds only because no always-compiled domain has a real code
//! edge into this tree. If one ever gains a real `use` of `flows::`, this
//! family must convert to the facade+stub shape (see `voice/`).

pub mod agents;
pub mod builder_tools;
pub mod bus;
pub mod catalogue;
pub mod discovery_tools;
mod draft_store;
pub mod memory_tools;
pub mod node_contracts;
pub mod ops;
mod schemas;
/// Skills this domain ships inside the binary. Needs BOTH gates: the pages
/// teach flows authoring, and `BundledSkill` is part of the skills subsystem.
#[cfg(feature = "skills")]
pub mod skills;
mod store;
/// The tinyflows engine seam (formerly `crate::tinyflows`).
pub mod tinyflows;
pub mod tools;

// The saved-flow model, the cancellation registries and the format importers
// are `tinyflows-catalog`'s, not this host's — a graph's identity, revision
// history, runs, drafts and suggestions are the same shapes for anyone keeping
// a library of workflows. These aliases keep the existing
// `flows::types::…` / `flows::run_registry::…` / `flows::n8n_import::…` call
// sites resolving unchanged while the definitions live upstream.
pub use tinyflows_catalog::import::n8n as n8n_import;
pub use tinyflows_catalog::{build_registry, run_registry, types};

pub use schemas::{
    all_controller_schemas as all_flows_controller_schemas,
    all_registered_controllers as all_flows_registered_controllers,
};
// `kv_get`/`kv_set` are re-exported (not just `pub(crate)`-visible within this
// domain's own module tree) because `tinyflows_sqlite::flows::SqliteStateStore`
// (built in `crates/openhuman-core/src/flows/tinyflows/caps/ops.rs`) lives in a sibling module and needs
// them to implement `tinyflows::caps::StateStore` without duplicating the
// `flow_state` table's persistence logic.
// `upsert_flow_run_step` is likewise re-exported for the tinyflows seam: the
// live run observer (`tinyflows::observability::FlowRunObserver`, issue G2)
// lives in the sibling `tinyflows` domain and persists each finished step onto
// the `flow_runs` row through this function as the run executes.
pub use node_contracts::{
    all_node_kind_contracts, node_kind_contract, render_node_kinds_line,
    render_node_kinds_required, ConfigField, NodeKindContract, PortSpec, NODE_KINDS,
};
pub use store::{kv_get, kv_set, upsert_flow_run_step};
pub use tinyflows_catalog::{
    DraftOrigin, Flow, FlowConnection, FlowDraft, FlowImport, FlowRevision, FlowRun, FlowRunStep,
    FlowRunTrigger, FlowSuggestion, FlowValidation, FlowValidationError, SuggestionStatus,
};
// Flow memory scoping (`flow:<id>` tags over memory v2) lives in
// `memory_tools`, the sibling that owns the agent tools; the digest
// subscriber, `flows_delete` and the tinyflows `memory` node adapter reach the
// same helpers through these re-exports so every caller tags identically.
pub use memory_tools::{
    cross_flow_filter, flow_filter, flow_key_of, flow_key_tag, flow_meta, flow_tag,
    forget_matching, remember_keyed, FLOWS_TAG,
};
