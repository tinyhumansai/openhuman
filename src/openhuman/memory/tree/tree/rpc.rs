//! RPC handler functions for the memory tree layer.
//!
//! Public JSON-RPC surface:
//! - `openhuman.memory_tree_ingest` — one unified ingest. Caller supplies
//!   `source_kind` + generic JSON `payload` (adapter-specific). Chat and
//!   document are canonicalised into contract items and handed to the bound
//!   driver's `Ingest` family; mail still canonicalises in process, for the
//!   reasons on [`ingest_rpc`].
//! - `openhuman.memory_tree_list_chunks` — listing with filters.
//! - `openhuman.memory_tree_get_chunk` — single chunk fetch.

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
include!("rpc_part_01.rs");
include!("rpc_part_02.rs");
include!("rpc_part_03.rs");
