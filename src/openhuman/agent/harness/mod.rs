//! Multi-agent harness — sub-agent dispatch and fork-cache support.
//!
//! ## Subagents-as-tools
//! The main agent runs its normal tool loop and can choose to delegate to a
//! sub-agent at any iteration via the `spawn_subagent` tool. The sub-agent
//! is constructed at call time from an [`definition::AgentDefinition`]
//! looked up in the global [`definition::AgentDefinitionRegistry`], runs
//! its own narrowed tool loop (cheaper model, fewer tools, no memory
//! recall), and returns a single text result that the parent threads back
//! into its history. This is the only execution shape — there is no
//! separate DAG planner/executor.
//!
//! ## Fork-cache mode
//! `spawn_subagent { mode: "fork", … }` replays the parent's *exact*
//! rendered system prompt + tool schemas + message prefix via the
//! [`fork_context::ForkContext`] task-local. The OpenAI-compatible
//! inference backend's automatic prefix caching turns this byte-stable
//! replay into a real token-savings win.
//!
//! ## Built-in agents
//! The canonical list of built-in agents lives in
//! [`crate::openhuman::agent::agents`] — one subfolder per agent, each
//! containing `agent.toml` (id, tools, model, sandbox, iteration cap)
//! and `prompt.md` (the sub-agent's system prompt body). Adding a new
//! built-in agent = drop in a new subfolder and append one entry to
//! that module's `BUILTINS` slice. [`builtin_definitions`] in this
//! harness module is a thin wrapper that loads those files and appends
//! the synthetic `fork` definition (used for prefix-cache reuse).

pub(crate) mod archivist;
pub(crate) mod builtin_definitions;
mod credentials;
pub mod definition;
pub(crate) mod definition_loader;
pub mod fork_context;
mod instructions;
pub mod interrupt;
pub(crate) mod memory_context;
mod parse;
pub(crate) mod self_healing;
pub mod session;
pub(crate) mod session_queue;
pub mod subagent_runner;
mod tool_loop;

pub use definition::{
    AgentDefinition, AgentDefinitionRegistry, DefinitionSource, ModelSpec, PromptSource,
    SandboxMode, ToolScope,
};
pub use fork_context::{
    current_fork, current_parent, with_fork_context, with_parent_context, ForkContext,
    ParentExecutionContext,
};
pub use interrupt::{check_interrupt, InterruptFence, InterruptedError};
pub use subagent_runner::{run_subagent, SubagentRunError, SubagentRunOptions};

pub(crate) use instructions::build_tool_instructions;
pub(crate) use parse::parse_tool_calls;
pub(crate) use tool_loop::run_tool_call_loop;

#[cfg(test)]
mod tests;
