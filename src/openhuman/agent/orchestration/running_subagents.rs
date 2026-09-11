//! Registry of in-flight async sub-agents that can be **steered** mid-run.
//!
//! `spawn_async_subagent` runs a child as a detached `tokio` task. On its own
//! that task is opaque: the parent gets a `task_id` back but has no channel into
//! the running loop and no way to collect the result inline. This registry
//! closes both gaps.
//!
//! Each running async sub-agent registers in TinyAgents'
//! [`DetachedTaskRegistry`], keyed by its `task_id`, with:
//! - an `Arc<RunQueue>` — the same steering channel the steering forwarder in
//!   `run_turn_via_tinyagents_shared` drains mid-turn, so `steer_subagent` can
//!   inject a message when no crate-native steering handle is registered;
//! - a TinyAgents `SteeringHandle` in the process-local
//!   `SteeringRegistry` while the child TinyAgents run is active, so
//!   steer/collect controls can deliver directly to the crate queue;
//! - a `watch::Receiver<SubagentStatus>` — so `wait_subagent` can block until the
//!   child reaches a terminal status;
//! - an `AbortHandle` — used by `subagent_cancel`/`close_subagent` paths to stop
//!   detached work.
//!
//! TinyAgents owns the process-local watch/cancel/abort/steering mechanics.
//! OpenHuman retains product metadata, durable task-store projection, and the
//! legacy `RunQueue` steering fallback. Ownership is enforced by parent session;
//! terminal entries are pruned on `wait` and swept at the registry soft cap.
//!
//! ## Typed lifecycle ledger (issue #4249)
//!
//! Alongside the executor plumbing (abort handle + steering queue + watch
//! status), every detached sub-agent is also recorded in a process-wide
//! [`tinyagents` orchestration `TaskStore`](crate::openhuman::agent::tinyagents::orchestration)
//! as an `OrchestrationTaskKind::SubAgent` task. `register` inserts it
//! (`Pending` → `Running`) and spawns a watcher that mirrors the child's
//! terminal status into the store (`Completed`/`Failed`/`Awaiting`); the cancel
//! paths record `Cancelled`. This gives a typed, queryable lifecycle
//! (`task_records`) alongside the crate-owned runtime registry.

#[cfg(test)]
#[path = "running_subagents_tests.rs"]
mod tests;
include!("running_subagents_part_01.rs");
include!("running_subagents_part_02.rs");
