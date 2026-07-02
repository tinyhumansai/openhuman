//! Shared agent-turn seams reused by the tinyagents harness route.
//!
//! The harness historically carried three near-identical agentic loops
//! (`Agent::turn`, `run_tool_call_loop`, the sub-agent `run_inner_loop`), all
//! retired in favour of the tinyagents harness (issue #4249). What survives here
//! are the cross-cutting pieces the tinyagents route still reuses: the
//! max-iteration [`CheckpointStrategy`] seam and the [`ProgressReporter`] /
//! [`TurnProgress`] sink that mirrors a turn's events onto `AgentProgress`.

pub(crate) mod checkpoint;
pub(crate) mod progress;

pub(crate) use checkpoint::{CheckpointOutcome, CheckpointStrategy};
pub(crate) use progress::{ProgressReporter, TurnProgress};
