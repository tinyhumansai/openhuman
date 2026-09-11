//! The agent's long-term goals.
//!
//! # The split this domain landed on (#5560)
//!
//! It used to have none: the store was `tinycortex::memory::goals`, reached
//! in-process, and everything above it — the RPC surface, the reflection agent,
//! the agent tools — named host types around that one call. With the engine
//! behind the loaded module, the contract splits the domain in the one place it
//! can be split without moving policy across a trait boundary:
//!
//! - **Driver** ([`MemoryGoals`](crate::openhuman::memory::api::provider::MemoryGoals),
//!   two members): read the document, replace the document. With it go
//!   persistence, the symlink-escape check, and the item-count and byte-size
//!   caps — all facts about a store, which is the thing a driver owns.
//! - **Host** ([`doc`]): parse, validate, mutate. `set_goals`' own contract
//!   docs say why the mutation surface may not be a driver's: the secret and
//!   PII predicates it runs are safety policy, and a per-item mutation member
//!   would put that policy behind a trait a third-party driver implements,
//!   where it could be skipped.
//!
//! [`ops`] is where those two halves meet — it holds the read-modify-write and
//! the lock that serialises it. Everything else here is unchanged: [`enrich`]
//! still runs the real `goals_agent`, and [`schemas`] still publishes the same
//! `memory_goals.*` wire shape.

pub mod doc;
pub mod enrich;
pub mod ops;
pub mod schemas;

pub use enrich::{enrich_goals, spawn_enrich_goals, GOALS_AGENT_ID};
pub use schemas::{all_memory_goals_controller_schemas, all_memory_goals_registered_controllers};
