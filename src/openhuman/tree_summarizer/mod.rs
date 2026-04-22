//! Hierarchical time-based summary tree.
//!
//! Organizes summaries as a tree: root → year → month → day → hour (leaf).
//! Each hour, a background job drains buffered raw content, summarizes it into
//! the hour leaf, and propagates updated summaries upward through the tree.
//! Stored as markdown files in `memory/namespaces/{ns}/tree/`.

pub mod bus;
pub mod cli;
pub mod engine;
pub mod ops;
pub mod store;
pub mod types;

mod schemas;

pub use ops as rpc;
pub use schemas::{
    all_controller_schemas as all_tree_summarizer_controller_schemas,
    all_registered_controllers as all_tree_summarizer_registered_controllers,
};
pub use types::*;
