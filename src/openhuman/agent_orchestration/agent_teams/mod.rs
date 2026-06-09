//! Durable agent-team coordination (issue #3374).
//!
//! A first-class, restart-survivable model for a lead agent coordinating a team
//! of worker agents: teams, members, dependency-aware tasks with race-safe
//! atomic claiming, and teammate messaging. All durable state lives in
//! `session_db::run_ledger` (the `agent_teams` / `agent_team_members` /
//! `agent_team_tasks` tables, plus the shared run-event log for messages),
//! never in the main chat context — so a coordination session can be listed,
//! inspected, and resumed.
//!
//! PR1 scope (this module today): the durable model + 8 read/write controllers
//! (`create`, `list`, `get`, `assign_task`, `claim_task`, `message_member`,
//! `list_messages`, `close`), the atomic compare-and-swap claim primitive, and
//! dependency validation (self / unknown / cycle). Live agent execution
//! (spawning workers, driving the run loop) and the UI are follow-up PRs.
//!
//! Namespace note: `agent_team` is distinct from the existing `team` domain,
//! which manages backend org/team membership.

pub mod ops;
mod schemas;
pub mod types;

pub use ops::{
    assign_task, claim_task, close_team, create_team, get_team, list_messages, list_teams,
    message_member, NewMember,
};
pub use schemas::{
    all_controller_schemas as all_agent_team_controller_schemas,
    all_registered_controllers as all_agent_team_registered_controllers,
};
pub use types::{TeamError, TeamView};
