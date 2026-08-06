//! Memory agent domain — owns the memory retrieval agent, its prompt, benchmarking,
//! and performance instrumentation for memory tree walking and chunk retrieval.
//!
//! The memory agent is a specialist that navigates the user's memory tree,
//! combining vector search, keyword matching, entity lookup, and hierarchical
//! tree browsing to answer queries. This domain centralizes the agent definition,
//! prompt construction, and retrieval performance tracking.

// `module_inception` is a byproduct of the domain-family reorg: the parent was
// renamed from `agent_memory` to `memory/agent`, which shortened it to match this
// long-standing inner module. Renaming the inner module would be a real rename
// on top of a pure move, so it is allowed here and left as follow-up.
#[allow(clippy::module_inception)]
pub mod agent;
pub mod memory_loader;
pub mod ops;
pub mod tools;
pub mod types;
