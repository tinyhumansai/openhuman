//! Unified agent turn engine.
//!
//! Historically the harness carried THREE near-identical agentic loops — one
//! per entry point (`Agent::turn` for web/desktop chat, `run_tool_call_loop`
//! for non-web channels + triage, and the subagent `run_inner_loop`). They each
//! re-implemented the same shape (call the LLM → parse tool calls → execute
//! tools → append results → repeat until final text or the iteration cap) and
//! had drifted in subtle ways.
//!
//! This module is the single home for the pieces those loops share, so they
//! can't drift again. The extraction is incremental (see the unify-agent-turn
//! plan): the first piece to land is [`tools::run_one_tool`] — the per-call
//! tool executor (policy gate → scope guard → approval gate → execute with
//! timeout → scrub/tokenjuice/cap/summarize → audit), which was previously
//! duplicated verbatim across all three loops.

pub(crate) mod checkpoint;
pub(crate) mod core;
pub(crate) mod parser;
pub(crate) mod progress;
pub(crate) mod state;
pub(crate) mod tool_source;
pub(crate) mod tools;

pub(crate) use checkpoint::{CheckpointOutcome, CheckpointStrategy, ErrorCheckpoint};
pub(crate) use core::run_turn_engine;
pub(crate) use parser::{DefaultParser, DispatcherParser};
pub(crate) use progress::{ProgressReporter, SubagentProgress, TurnProgress};
pub(crate) use state::{NullObserver, TurnObserver};
pub(crate) use tool_source::{RegistryToolSource, ToolSource};
pub(crate) use tools::{run_one_tool, ToolRunResult};
