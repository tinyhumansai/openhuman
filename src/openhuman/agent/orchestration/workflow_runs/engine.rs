//! Live execution engine for durable workflow runs (#3375 PR2).
//!
//! PR1 shipped the declarative [`WorkflowDefinition`] model, the durable
//! [`WorkflowRun`] ledger, and the read controllers. This module makes runs
//! actually *execute*: [`start_workflow_run`] resolves a definition, creates a
//! `Running` ledger row, and spawns a non-blocking engine task that walks the
//! phase DAG in dependency order, fanning out each phase's agents through the
//! programmatic [`AgentOrchestrationSession`] with bounded concurrency, then
//! persisting phase outputs after every phase. [`stop_workflow_run`] flips a
//! cancellation signal the loop checks between phases (→ `Interrupted`);
//! [`resume_workflow_run`] reloads a run and continues from the first
//! incomplete phase.
//!
//! ## Root parent context (the one real unknown)
//!
//! Child agents are spawned via [`AgentOrchestrationSession::spawn_agent`],
//! which reads the *parent execution context* from a task-local
//! ([`current_parent`]). The engine runs from a controller-spawned background
//! task — there is **no** agent turn on the stack, so the task-local is unset
//! and a naive spawn would fail with `NoParentContext`.
//!
//! The fix mirrors the production blueprint in
//! [`crate::openhuman::agent::triage::escalation`]: build a *root*
//! [`ParentExecutionContext`] from a real [`Agent`] (`Agent::from_config`) and
//! run the whole phase loop inside [`with_parent_context`]. Every
//! `spawn_agent` call nested in that scope then resolves `current_parent()` to
//! the root, inheriting a real provider, tool registry, memory, and model — the
//! same construction path `agent_chat` uses.
//!
//! ## TODO(#4249, 08.3): human-review phases as durable interrupts
//!
//! When a workflow phase gains a *human-review* gate, express the pause as a
//! durable graph interrupt (`NodeResult::Interrupt` persisted via the
//! checkpointer, resumed with `Command { resume: .. }`) instead of the ad-hoc
//! `Interrupted`/cancel-flag bookkeeping used for stop/resume here. The
//! mechanism is already implemented end-to-end for the delegation review gate in
//! [`crate::openhuman::agent::tinyagents::delegation`] (see `run_delegation_durable` /
//! `resume_delegation`); this engine should adopt the same
//! interrupt→checkpoint→resume path once a human-review phase kind exists. The
//! current between-phase cancellation bookkeeping is intentionally left in place
//! until that phase kind lands, to keep stop/resume semantics unchanged.

#[cfg(test)]
#[path = "engine_tests.rs"]
mod engine_tests;
include!("engine_part_01.rs");
include!("engine_part_02.rs");
