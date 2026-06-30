//! `memory_goals` — the agent's long-term goals when interacting with the
//! user.
//!
//! A deliberately small, high-level domain: it maintains a compact markdown
//! file (`MEMORY_GOALS.md`, ~200–500 tokens) holding an editable **list** of
//! the user's durable goals. The list can be mutated three ways:
//!
//! - **Explicitly** — via RPC (`openhuman.memory_goals_{list,add,edit,delete}`)
//!   or the matching agent tools (`goals_list` / `goals_add` / `goals_edit` /
//!   `goals_delete`).
//! - **By reflection** — a turn-based [`enrich`]ment agent (`goals_agent`) that
//!   reads context + memory and applies add/edit/delete over several turns. On
//!   an empty list it performs an initial population.
//! - **Automatically** — the reflection agent is fired (best-effort) when the
//!   conversation context is summarized; see the archivist segment-close hook.
//!
//! Persistence + cap enforcement live in [`store`]; the file is stored state,
//! not injected into the main system prompt.

pub mod enrich;
pub mod ops;
mod schemas;
pub mod store;
pub mod tools;
pub mod types;

pub use enrich::{enrich_goals, spawn_enrich_goals, GOALS_AGENT_ID};
pub use schemas::{all_memory_goals_controller_schemas, all_memory_goals_registered_controllers};
pub use tools::{GoalsAddTool, GoalsDeleteTool, GoalsEditTool, GoalsListTool};
pub use types::{GoalItem, GoalsDoc};
