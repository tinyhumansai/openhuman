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
//! Scope (this module today): the durable model + 10 read/write controllers
//! (`create`, `list`, `get`, `assign_task`, `claim_task`, `message_member`,
//! `list_messages`, `complete_task`, `shutdown_member`, `close`), the atomic
//! compare-and-swap claim primitive, dependency validation (self / unknown /
//! cycle), and quality-gated task completion (dependencies done, claimant owns
//! the task, evidence present when required). Live agent execution (spawning
//! workers, driving the run loop) and the message-send UI are a follow-up PR.
//!
//! Namespace note: `agent_team` is distinct from the existing `team` domain,
//! which manages backend org/team membership.

pub mod ops;
mod schemas;
pub mod types;

pub use ops::{
    assign_task, claim_task, close_team, complete_task, create_team, get_team, list_messages,
    list_teams, message_member, shutdown_member, NewMember,
};
pub use schemas::{
    all_controller_schemas as all_agent_team_controller_schemas,
    all_registered_controllers as all_agent_team_registered_controllers,
};
pub use types::{MemberShutdown, TeamError, TeamView};
