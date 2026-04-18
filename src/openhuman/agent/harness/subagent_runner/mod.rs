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
//! 5. Runs a slim inner tool-call loop using the parent's
//!    [`crate::openhuman::providers::Provider`] and returns a single
//!    text result. The intra-sub-agent history never leaks back to the
//!    parent — the parent only sees one compact tool result.
//!
//! ## Layout
//!
//! This is a light `mod.rs`: every item below is declared in a sibling
//! file and re-exported here.
//!
//! | File              | Contents                                                    |
//! | ----------------- | ----------------------------------------------------------- |
//! | `types.rs`        | `SubagentRun{Options,Outcome,Error}`, `SubagentMode`        |
//! | `ops.rs`          | `run_subagent`, typed + fork mode, inner tool-call loop     |
//! | `handoff.rs`      | Oversized-tool-result cache + hygiene helpers               |
//! | `extract_tool.rs` | `extract_from_result` tool (direct provider extraction)     |
//! | `tool_prep.rs`    | Tool filtering + prompt loading + text-mode protocol block  |

mod extract_tool;
mod handoff;
mod ops;
mod tool_prep;
mod types;

// Public API — the entry point and the shapes it returns.
pub use ops::run_subagent;
pub use types::{SubagentMode, SubagentRunError, SubagentRunOptions, SubagentRunOutcome};

// Crate-internal re-exports: `debug_dump` calls the text-mode protocol
// renderer, and `session::builder` reuses the welcome-only guard. The
// other `tool_prep` helpers are used only inside this module.
pub(crate) use tool_prep::{build_text_mode_tool_instructions, is_welcome_only_tool};
