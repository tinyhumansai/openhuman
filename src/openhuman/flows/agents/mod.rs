//! Flow-domain agents: the specialist agents that author and discover flows.
//!
//! These are first-class built-in agents (registered in the global
//! `agent_registry` via [`crate::openhuman::agent::registry::agents::loader`]),
//! but their definitions live here — inside the flows high-level module —
//! alongside the flows data model, tools, and RPCs they operate on, so the whole
//! feature is one cohesive unit. The loader's `BUILTINS` slice points at these
//! modules by path (the same cross-module pattern `reasoning_agent` uses from
//! `orchestration/`), so registration stays centralized while ownership stays
//! with the domain.
//!
//! - [`workflow_builder`] — authors tinyflows [`WorkflowGraph`](tinyflows::model::WorkflowGraph)s
//!   from natural language and returns a validated PROPOSAL by default. It
//!   also carries three narrow, deliberate persistence tools —
//!   `create_workflow`/`duplicate_flow` (always born DISABLED) and
//!   `save_workflow` (update-only, onto a flow that already exists) — each
//!   gated behind the user's explicit ask; it never enables a flow. See the
//!   module doc in `flows::builder_tools` for the full tool-by-tool table.
//! - [`flow_discovery`] — the read-only "Flow Scout" that grounds buildable
//!   automation ideas from the user's data.

pub mod flow_discovery;
pub mod workflow_builder;
