//! The `flows::` domain: saved automation workflows (TinyFlows graphs) —
//! create/get/list/update/delete/enable/run through the TinyFlows module and
//! its CatalogStorage callback. Mirrors `crates/openhuman-core/src/cron/`'s
//! module shape.
//!
//! Business logic lives in [`ops`]; workflow definitions in `store`; the RPC/CLI
//! controller surface in `schemas` (private, re-exported below).
//!
//! # Gate shape — leaf, not facade
//!
//! This domain is gated at
//! `pub mod flows;` in `crates/openhuman-core/src/lib.rs` on `#[cfg(feature = "flows")]`,
//! and the submodules below inherit that gate. There is **no `stub.rs`**:
//! every symbol reached from outside is a *registration site* (`core::all`,
//! `core::runtime::subscribers`' `FlowTriggerSubscriber`, `core::runtime::services`' boot
//! reconcile, the agent-tool `vec!` in `tools::ops`, the `workflow_builder` /
//! `flow_discovery` entries in `agent_registry`'s `BUILTINS`), and a registration site wants
//! *absence*, not a disabled-error stub — otherwise `flows.*` becomes a known
//! method that fails at runtime.
//!
//! The leaf gate holds only because no always-compiled domain has a real code
//! edge into this tree. If one ever gains a real `use` of `flows::`, this
//! family must convert to the facade+stub shape (see `voice/`).

mod agent_route;
pub(crate) mod authoring_host;
pub mod builder_host;
pub(crate) mod builder_tools;
mod builder_tools_connection_reads;
pub mod bus;
pub(crate) mod caps_host;
pub(crate) mod catalog_storage;
pub mod catalogue;
mod copilot_bus;
mod draft_store;
pub(crate) mod host_gates;
pub mod http_credentials;
pub mod memory_tools;
pub mod node_contracts;
pub mod ops;
pub(crate) mod recovery;
pub(crate) mod registration;
pub(crate) mod runs_adapter;
mod schemas;
mod store;
#[cfg(test)]
pub(crate) mod test_support;
pub use schemas::{
    all_controller_schemas as all_flows_controller_schemas,
    all_registered_controllers as all_flows_registered_controllers,
};
pub use {node_contracts::all_node_kind_contracts, node_contracts::node_kind_contract};
// Flow memory scoping (`flow:<id>` tags over memory v2) lives in
// `memory_tools`, the sibling that owns the agent tools; the digest
// subscriber, `flows_delete` and the tinyflows `memory` node adapter reach the
// same helpers through these re-exports so every caller tags identically.
pub use memory_tools::{
    cross_flow_filter, flow_filter, flow_key_of, flow_key_tag, flow_meta, flow_tag,
    forget_matching, remember_keyed, FLOWS_TAG,
};
