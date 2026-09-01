//! Host layer over the engine's tool-memory domain.
//!
//! The rule store and its types are the engine's. What stays here is the pair
//! that plugs into the agent harness — the post-turn capture hook and the
//! system prompt section — both of which name harness traits (`PostTurnHook`,
//! `PromptContext`) that the engine crate cannot see.
//!
//! # Where the four names in this module came from (#5560)
//!
//! This module used to be `pub use tinymemory_core::tool_memory::*;`, and that
//! glob resolved to four different places:
//!
//! | Name | Where it used to live | Where it is now |
//! | --- | --- | --- |
//! | `ToolMemoryStore`, `TOOL_MEMORY_PROMPT_CAP` | `tinycortex::memory::tool_memory::store` | [`store`], in this directory |
//! | `tool_memory_namespace`, `ToolMemoryPriority`, `ToolMemoryRule`, `ToolMemorySource` | `tinycortex::memory::tool_memory::types` | `tinymemory_api::tool_memory` — the contract |
//! | `tool_memory_store` | a ten-line constructor in `tinymemory-core` | [`tool_memory_store`] below |
//! | `test_helpers` | `tinymemory-core`, behind `cfg(any(test, feature = "test-support"))` | unchanged, and test-only |
//!
//! The **second** row is a pure repoint and changes no type. The engine's
//! `memory::tool_memory::types` is `tinycortex_api::tool_memory`, and
//! `tinycortex-api` is itself now nothing but `pub use tinymemory_api::{…}` —
//! so the contract path and the engine path name the *same items*, and the
//! engine's own `MemoryToolMemory` implementation serialises those very types.
//! Naming the contract removes an alias, not an indirection.
//!
//! The **first** row came home rather than being repointed, because there was
//! nothing to repoint it at: the contract has the vocabulary a rule is made of
//! but not the store that files one. [`store`] says at length why it is a
//! host-side convention over `Arc<dyn Memory>` rather than a call onto
//! `MemoryToolMemory` — the short version is that both of its callers are
//! handed a *subtree-scoped* memory object, and the family reached through the
//! ambient guard is the shared tree.
//!
//! The **third** row came home earlier for the same reason:
//! [`tool_memory_store`] below is the same one-line `ToolMemoryStore::new`
//! wrapper, and it was only in the engine crate because that crate used to be
//! this host's memory layer. Nothing in `tinymemory` named it.
//!
//! The fourth row is **test-only** and stays on the engine crate deliberately:
//! `MockMemory` is a test fixture reached from four inline `#[cfg(test)]`
//! modules, and `cfg(test)` code links against the `tinymemory-core`
//! **dev-dependency** (declared with `features = ["test-support"]`), which
//! survives the shed. It is not a production reference and does not keep the
//! engine crate in the shipped binary. It now sits in `test_support/` — a
//! directory both memory lints skip by path, and `#[cfg(test)]` so it is not
//! link-resolvable from a docs build — which is why the one line of this file
//! that named the engine crate no longer reads as an unmigrated production
//! call. See that module for why the classification is the point.

use std::sync::Arc;

// The contract's storage trait, named at the contract rather than through
// `memory::Memory` — same item, one less alias to follow.
use tinymemory_api::traits::Memory;

// The rule vocabulary, named at the contract. Re-exported rather than merely
// available so every historical `memory::tool_memory::ToolMemoryRule` path
// keeps resolving — and resolving to the item that actually crosses the bus.
pub use tinymemory_api::tool_memory::{
    tool_memory_namespace, ToolMemoryPriority, ToolMemoryRule, ToolMemorySource,
};

pub use store::{ToolMemoryStore, TOOL_MEMORY_PROMPT_CAP};

pub mod capture;
pub mod prompt;
pub mod store;

/// Build the rule store over OpenHuman's shared memory object.
///
/// A named constructor rather than `ToolMemoryStore::new` at each call site:
/// every caller (`capture.rs`, the harness session builder, the raw-coverage
/// suite) passes the same `Arc<dyn Memory>` the host already holds, and the
/// indirection is what let the store's own construction move between crates
/// without touching them. Kept for exactly that reason.
pub fn tool_memory_store(memory: Arc<dyn Memory>) -> ToolMemoryStore {
    log::trace!("[memory::tool_memory] building ToolMemoryStore over the host memory object");
    ToolMemoryStore::new(memory)
}

// The engine crate's `MockMemory` fixture, re-exported under its historical
// path `memory::tool_memory::test_helpers`.
//
// Test-only in both directions: `tinymemory-core` compiles it behind
// `cfg(any(test, feature = "test-support"))`, and both items below are
// `#[cfg(test)]`, so they exist only in `cargo test --lib` builds where the
// dev-dependency at `Cargo.toml`'s `[dev-dependencies]` supplies the crate.
// Four inline `#[cfg(test)]` modules reach it — `capture` in this directory,
// `agent::experience::{capture, store}` and
// `agent::tinyagents::host::experience_store`.
//
// The `pub use tinymemory_core::…` line itself sits one level down, in
// `test_support/mod.rs`; that module's docs say why, and the path callers use
// does not change.
#[cfg(test)]
pub mod test_support;

#[cfg(test)]
pub use test_support::test_helpers;
