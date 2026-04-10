//! Multi-agent harness — sub-agent dispatch and the orchestrator topology.
//!
//! Two execution shapes coexist here:
//!
//! ## Subagents-as-tools (default)
//! The main agent runs its normal tool loop and can choose to delegate to a
//! sub-agent at any iteration via the `spawn_subagent` tool. The sub-agent
//! is constructed at call time from an [`definition::AgentDefinition`]
//! looked up in the global [`definition::AgentDefinitionRegistry`], runs
//! its own narrowed tool loop (cheaper model, fewer tools, no memory
//! recall), and returns a single text result that the parent threads back
//! into its history. This is the recommended shape for interactive use.
//!
//! ## DAG orchestration (opt-in via `OrchestratorConfig::enabled`)
//! A pre-existing planner→DAG→execute→synthesise loop in
//! [`executor::run_orchestrated`]. Useful for batch scenarios but heavier
//! than the tool-call path. As of the subagent refactor it shares the same
//! [`subagent_runner::run_subagent`] helper internally.
//!
//! ## Fork-cache mode
//! Both shapes can request a `fork`-mode sub-agent that replays the
//! parent's *exact* rendered system prompt + tool schemas + message
//! prefix via the [`fork_context::ForkContext`] task-local. The
//! OpenAI-compatible inference backend's automatic prefix caching turns
//! this byte-stable replay into a real token-savings win.
//!
//! ## Built-in archetypes
//! Eight historical archetypes are preserved and surfaced as built-in
//! definitions in [`builtin_definitions`]:
//!
//! 1. **Orchestrator** — routes, judges quality, synthesises.
//! 2. **Planner** — breaks goals into a DAG of subtasks.
//! 3. **Code Executor** — writes & runs code in a sandbox.
//! 4. **Skills Agent** — executes QuickJS skill tools.
//! 5. **Tool-Maker** — self-heals missing commands with polyfill scripts.
//! 6. **Researcher** — reads real documentation, compresses to markdown.
//! 7. **Critic** — adversarial QA review.
//! 8. **Archivist** — background post-session knowledge extraction.

pub mod archetypes;
pub mod archivist;
pub mod builtin_definitions;
pub mod context_assembly;
pub mod dag;
pub mod definition;
pub mod definition_loader;
pub mod executor;
pub mod fork_context;
pub mod interrupt;
pub mod self_healing;
pub mod session_queue;
pub mod subagent_runner;
pub mod types;

pub use archetypes::AgentArchetype;
pub use archivist::ArchivistHook;
pub use dag::{DagError, TaskDag, TaskNode};
pub use definition::{
    AgentDefinition, AgentDefinitionRegistry, DefinitionSource, ModelSpec, PromptSource,
    SandboxMode, ToolScope,
};
pub use executor::run_orchestrated;
pub use fork_context::{
    current_fork, current_parent, with_fork_context, with_parent_context, ForkContext,
    ParentExecutionContext,
};
pub use interrupt::{check_interrupt, InterruptFence, InterruptedError};
pub use self_healing::SelfHealingInterceptor;
pub use session_queue::SessionQueue;
pub use subagent_runner::{run_subagent, SubagentRunError, SubagentRunOptions};
pub use types::*;
