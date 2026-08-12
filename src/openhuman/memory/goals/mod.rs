//! The agent's long-term goals.
//!
//! Unlike its sibling domains this one did **not** split across the memory
//! extraction: its store lives in `tinycortex::memory::goals`, and everything
//! above that — the RPC surface, the reflection agent, the agent tools — names
//! host types. `tinymemory-core` briefly carried an empty `goals` module as a
//! result; it was removed rather than left as a husk that re-exported nothing.

pub mod enrich;
pub mod ops;
pub mod schemas;

pub use enrich::{enrich_goals, spawn_enrich_goals, GOALS_AGENT_ID};
pub use schemas::{all_memory_goals_controller_schemas, all_memory_goals_registered_controllers};
