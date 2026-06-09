//! Aggregate + validation types for durable agent-team coordination (#3374).
//!
//! The durable row types ([`AgentTeam`], [`AgentTeamMember`], [`AgentTeamTask`],
//! [`ClaimOutcome`]) live in `session_db::run_ledger`. This module adds the
//! read-aggregate view returned by the controllers and the validation error
//! surface used by `ops::assign_task`.

use serde::Serialize;

use crate::openhuman::session_db::run_ledger::{AgentTeam, AgentTeamMember, AgentTeamTask};

/// A team plus its members and tasks — the shape returned by `get`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamView {
    pub team: AgentTeam,
    pub members: Vec<AgentTeamMember>,
    pub tasks: Vec<AgentTeamTask>,
}

/// A validation / coordination problem surfaced by the agent-team ops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "detail")]
pub enum TeamError {
    /// A member name collides with an existing member in the same team.
    DuplicateMemberName { name: String },
    /// A referenced member id is not part of the team.
    UnknownMember { member_id: String },
    /// A task declared itself as one of its own dependencies.
    SelfDependency { task_id: String },
    /// A dependency edge would introduce a cycle among the team's tasks.
    CyclicDependency,
    /// A dependency named a task id that does not exist in the team.
    UnknownDependency { depends_on: String },
}

impl std::fmt::Display for TeamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TeamError::DuplicateMemberName { name } => {
                write!(f, "duplicate member name: {name}")
            }
            TeamError::UnknownMember { member_id } => {
                write!(f, "unknown member: {member_id}")
            }
            TeamError::SelfDependency { task_id } => {
                write!(f, "task {task_id} cannot depend on itself")
            }
            TeamError::CyclicDependency => write!(f, "dependency cycle detected"),
            TeamError::UnknownDependency { depends_on } => {
                write!(f, "unknown dependency: {depends_on}")
            }
        }
    }
}

impl std::error::Error for TeamError {}
