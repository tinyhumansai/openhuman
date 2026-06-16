//! Memory agent domain — owns the memory retrieval agent, its prompt, benchmarking,
//! and performance instrumentation for memory tree walking and chunk retrieval.
//!
//! The memory agent is a specialist that navigates the user's memory tree,
//! combining vector search, keyword matching, entity lookup, and hierarchical
//! tree browsing to answer queries. This domain centralizes the agent definition,
//! prompt construction, and retrieval performance tracking.

pub mod agent;
pub mod memory_loader;
pub mod ops;
pub mod tools;
pub mod types;
