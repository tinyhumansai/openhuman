//! Thin OpenHuman host adapters for [`tinyagents_graph::goals`].
//!
//! Tinyagents owns the goal types, lifecycle, persistence, prompt rendering,
//! graph continuation, and native harness tools. This compatibility module
//! contains only OpenHuman-specific concerns: workspace-store resolution,
//! JSON-RPC schemas, domain events, the legacy file migration, heartbeat
//! dispatch, and adapters for OpenHuman's pre-tinyagents `Tool`/`StopHook`
//! traits. The external `thread_goals.*` RPC namespace remains stable.

pub mod continuation;
pub mod migration;
pub mod ops;
pub mod runtime;
mod schemas;
pub mod store;
pub mod tools;

pub use schemas::{all_thread_goals_controller_schemas, all_thread_goals_registered_controllers};
pub use tinyagents_graph::goals::{ThreadGoal, ThreadGoalStatus};
pub use tools::{GoalCompleteTool, GoalGetTool, GoalSetTool};
