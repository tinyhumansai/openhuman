//! Sub-agent execution runner.
//!
//! Given an [`super::definition::AgentDefinition`] and a task prompt, the
//! runner:
//!
//! 1. Reads the [`super::fork_context::ParentExecutionContext`] task-local
//!    set by the parent [`crate::openhuman::agent::Agent::turn`].
//! 2. Resolves the sub-agent's model name (inherit / hint / exact).
//! 3. Filters the parent's tool registry per `definition.tools`,
//!    `disallowed_tools`, and `skill_filter` (or, in `fork` mode,
//!    inherits the parent's tools verbatim).
//! 4. Builds a narrow system prompt that strips the sections the
//!    definition asks to omit (`omit_identity`, `omit_memory_context`,
//!    `omit_safety_preamble`, `omit_skills_catalog`).
//! 5. Runs the child turn on the TinyAgents harness (`ops::graph` →
//!    [`crate::openhuman::tinyagents::run_turn_via_tinyagents_shared`]) using
//!    the parent's [`crate::openhuman::inference::provider::Provider`], then
//!    mirrors the child transcript/progress and returns one compact tool result
//!    to the parent.
//!
//! This module is the OpenHuman **build pipeline** around that TinyAgents
//! run: definition lookup/allowlists, archetype prompt assembly, toolkit
//! filtering, sandbox/action-root narrowing, checkpoint/handback, and
//! worker-thread transcript mirroring. Mapping it onto TinyAgents
//! `SubAgent`/`SubAgentSession`/subgraph primitives is tracked in
//! `docs/tinyagents-full-migration-plan/07-subagents/`.
//!
//! ## Layout
//!
//! This is a light `mod.rs`: every item below is declared in a sibling
//! file and re-exported here.
//!
//! | File              | Contents                                                    |
//! | ----------------- | ----------------------------------------------------------- |
//! | `types.rs`        | `SubagentRun{Options,Outcome,Error}`, `SubagentMode`        |
//! | `ops/`            | `run_subagent`, typed/fork mode, TinyAgents graph route     |
//! | `handoff.rs`      | Oversized-tool-result cache + hygiene helpers               |
//! | `extract_tool.rs` | `extract_from_result` tool (direct provider extraction)     |
//! | `tool_prep.rs`    | Tool filtering + prompt loading + text-mode protocol block  |

mod autonomous;
mod extract_tool;
mod handoff;
mod ops;
mod tool_prep;
mod types;

// Public API — the entry point and the shapes it returns.
pub use autonomous::{
    autonomous_iter_cap, subagent_iter_cap_with_autonomous_lift, with_autonomous_iter_cap,
};
pub use ops::run_subagent;
pub use types::{
    SubagentCheckpointData, SubagentMode, SubagentRunError, SubagentRunOptions, SubagentRunOutcome,
    SubagentRunStatus, SubagentUsage,
};

// Crate-internal re-exports: `agent::debug` calls the text-mode protocol
// renderer. The other `tool_prep` helpers are used only inside this module.
pub(crate) use tool_prep::build_text_mode_tool_instructions;

// Progressive-disclosure handoff: the tinyagents `HandoffMiddleware` intercepts
// oversized sub-agent tool results via `apply_handoff`, sharing the per-spawn
// `ResultHandoffCache` with the `extract_from_result` tool.
pub(crate) use handoff::{apply_handoff, ResultHandoffCache};
pub(crate) use ops::run_agent_turn_request_via_default_graph;
pub(crate) use ops::{append_subagent_role_contract, resolve_subagent_source};

// `user_is_signed_in_to_composio` is the mode-aware "can the user call
// composio at all?" probe added in Wave 2 (#1710). Re-exported here so
// non-composio probe sites (registration gates, heartbeat telemetry)
// can call it as
// `crate::openhuman::agent::harness::subagent_runner::user_is_signed_in_to_composio`
// without reaching into a private sibling module.
pub(crate) use ops::user_is_signed_in_to_composio;
