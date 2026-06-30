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
//! Scope: the durable model + 11 read/write controllers (`create`, `list`,
//! `get`, `assign_task`, `claim_task`, `message_member`, `list_messages`,
//! `complete_task`, `shutdown_member`, `close`, `start_member`), the atomic
//! compare-and-swap claim primitive, dependency validation (self / unknown /
//! cycle), quality-gated task completion (dependencies done, claimant owns the
//! task, evidence present when required), and — as of #3374 PR4 — **live
//! teammate execution**: `start_member` (in [`runtime`]) claims a task and
//! spawns a real worker sub-agent that runs to completion, capturing its output
//! as the task's evidence, with pending lead/teammate messages delivered into
//! the worker prompt at spawn.
//!
//! Namespace note: `agent_team` is distinct from the existing `team` domain,
//! which manages backend org/team membership.

pub mod ops;
pub mod runtime;
mod schemas;
pub mod types;

pub use ops::{
    assign_task, claim_task, close_team, complete_task, create_team, get_team, list_messages,
    list_teams, message_member, shutdown_member, NewMember,
};
pub use runtime::start_member_run;
pub use schemas::{
    all_controller_schemas as all_agent_team_controller_schemas,
    all_registered_controllers as all_agent_team_registered_controllers,
};
pub use types::{MemberShutdown, StartMemberOutcome, TeamError, TeamView};
