//! Tool sourcing seam for the turn engine.
//!
//! The three former loops resolved "what tools can the model call this turn and
//! how do I execute one" differently:
//!
//! * the channel loop advertised `registry + extra` filtered by a visibility
//!   whitelist, and executed via the shared [`run_one_tool`];
//! * the subagent loop advertised a definition-filtered slice of the parent's
//!   tools (with lazy toolkit registration), and had its own per-call body;
//! * `Agent::turn` advertised `Agent.visible_tool_specs` and executed via the
//!   richer `Agent::execute_tool_call` (session policy + per-call permission
//!   levels + `execute_with_options`).
//!
//! [`ToolSource`] is the single seam the engine talks to: it advertises the
//! request specs and owns per-call execution (including the start/complete
//! progress events). [`RegistryToolSource`] is the channel/CLI/triage impl; the
//! subagent and `Agent` impls land in later phases.

use std::collections::HashSet;

use async_trait::async_trait;

use super::super::payload_summarizer::PayloadSummarizer;
use super::progress::ProgressReporter;
use super::{run_one_tool, ToolRunResult};
use crate::openhuman::agent::harness::parse::ParsedToolCall;
use crate::openhuman::tools::policy::ToolPolicy;
use crate::openhuman::tools::{Tool, ToolSpec};

/// What the engine needs from "the set of tools available this turn".
#[async_trait]
pub(crate) trait ToolSource: Send {
    /// The deduped, visibility-filtered specs to advertise to the provider
    /// this turn. Re-read each iteration so impls that register tools lazily
    /// (subagent toolkit resolution) can grow the advertised set over a turn.
    fn request_specs(&self) -> &[ToolSpec];

    /// Execute one parsed tool call end-to-end, emitting its `ToolCallStarted`
    /// / `ToolCallCompleted` (or flavor-equivalent) progress events. Returns a
    /// [`ToolRunResult`] the engine folds into history + the circuit breaker.
    async fn execute_call(
        &mut self,
        call: &ParsedToolCall,
        iteration: usize,
        progress: &dyn ProgressReporter,
        progress_call_id: &str,
    ) -> ToolRunResult;
}

/// The channel/CLI/triage tool source: a persistent `registry`, optional
/// per-turn synthesised `extra` tools, an optional visibility whitelist, and a
/// pluggable [`ToolPolicy`]. Mirrors the original `run_tool_call_loop` tool
/// plumbing exactly.
pub(crate) struct RegistryToolSource<'a> {
    registry: &'a [Box<dyn Tool>],
    extra: &'a [Box<dyn Tool>],
    visible: Option<&'a HashSet<String>>,
    tool_policy: &'a dyn ToolPolicy,
    payload_summarizer: Option<&'a dyn PayloadSummarizer>,
    specs: Vec<ToolSpec>,
}

impl<'a> RegistryToolSource<'a> {
    pub(crate) fn new(
        registry: &'a [Box<dyn Tool>],
        extra: &'a [Box<dyn Tool>],
        visible: Option<&'a HashSet<String>>,
        tool_policy: &'a dyn ToolPolicy,
        payload_summarizer: Option<&'a dyn PayloadSummarizer>,
    ) -> Self {
        // Filter to visible tools, then dedup by name before sending to the
        // provider. Registry tools may collide with per-turn synthesised
        // extra_tools (e.g. an `ArchetypeDelegationTool` whose
        // `delegate_name = "research"` shadowing a same-named skill). Some
        // providers 400 on duplicate tool names — see TAURI-RUST-4.
        let filtered: Vec<ToolSpec> = registry
            .iter()
            .chain(extra.iter())
            .filter(|tool| visible.map(|s| s.contains(tool.name())).unwrap_or(true))
            .map(|tool| tool.spec())
            .collect();
        let specs = crate::openhuman::agent::harness::session::dedup_visible_tool_specs(filtered);
        Self {
            registry,
            extra,
            visible,
            tool_policy,
            payload_summarizer,
            specs,
        }
    }

    fn is_visible(&self, name: &str) -> bool {
        self.visible.map(|s| s.contains(name)).unwrap_or(true)
    }
}

#[async_trait]
impl ToolSource for RegistryToolSource<'_> {
    fn request_specs(&self) -> &[ToolSpec] {
        &self.specs
    }

    async fn execute_call(
        &mut self,
        call: &ParsedToolCall,
        iteration: usize,
        progress: &dyn ProgressReporter,
        progress_call_id: &str,
    ) -> ToolRunResult {
        // Look up the tool by name in the combined registry + extras, subject
        // to the visibility whitelist. A hallucinated / filtered-out name
        // resolves to `None`, which `run_one_tool` reports as an unknown tool.
        let tool_opt: Option<&dyn Tool> = self
            .registry
            .iter()
            .chain(self.extra.iter())
            .find(|t| t.name() == call.name && self.is_visible(t.name()))
            .map(|b| b.as_ref());
        run_one_tool(
            tool_opt,
            call,
            iteration,
            progress,
            self.tool_policy,
            self.payload_summarizer,
            progress_call_id,
        )
        .await
    }
}
