//! `thread_goals` — the agent's single, thread-scoped goal.
//!
//! A **thread goal** is a durable "completion contract" the agent keeps
//! pursuing across turns, interrupts, resumes, and budget boundaries — modelled
//! on OpenAI Codex's `/goal`. It is deliberately distinct from the two existing
//! "goals" concepts:
//!
//! - [`memory_goals`](crate::openhuman::memory_goals) — a *global*, long-term
//!   list of the user's durable goals (`MEMORY_GOALS.md`).
//! - the per-thread kanban [task board](crate::openhuman::agent::task_board) —
//!   a list of work cards.
//!
//! There is **exactly one** thread goal per thread, with a small lifecycle
//! (active / paused / budget_limited / complete), an optional token budget, and
//! support for autonomous idle continuation.
//!
//! Persistence is per-thread file-JSON under
//! `<workspace>/thread_goals/<hex(thread_id)>.json` (see [`store`]). Two writers
//! may set it when a chat begins: the orchestrator (authoritative, via
//! `goal_set`) and the context-gathering path (proposes only if absent, via
//! [`store::set_if_absent`]).

pub mod continuation;
pub mod ops;
pub mod runtime;
mod schemas;
pub mod store;
pub mod tools;
pub mod types;

pub use schemas::{all_thread_goals_controller_schemas, all_thread_goals_registered_controllers};
pub use tools::{GoalCompleteTool, GoalGetTool, GoalSetTool};
pub use types::{ThreadGoal, ThreadGoalStatus};
