//! Read-only agent run replay + status RPC surface (workstream 05.x).
//!
//! Thin controllers over the C4 durable journal/status seams in
//! [`crate::openhuman::tinyagents::journal`]. Everything here is a reader:
//! no mutation, no writes, no security/approval/sandbox bypass. See
//! [`schemas`] for the three `agent`-namespace controllers and [`ops`] for the
//! workspace-parameterized read logic.

pub(crate) mod ops;
mod schemas;

pub(crate) use schemas::{
    all_agent_replay_controller_schemas, all_agent_replay_registered_controllers,
};
