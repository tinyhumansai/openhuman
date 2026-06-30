//! Sub-agent [`ToolSource`] implementation.
//!
//! Looks up tools in `extra_tools` then the parent registry, lazily registers
//! toolkit actions the fuzzy filter omitted, rejects names outside the
//! allowlist, and routes execution through the shared [`run_one_tool`] (so
//! sub-agents now get the same approval gate, audit, credential scrub,
//! tokenjuice and timeout as the channel loop), then applies the
//! progressive-disclosure handoff.

use std::collections::HashSet;

use crate::openhuman::tools::{Tool, ToolSpec};

use super::handoff_helper::apply_handoff;
use super::provider::LazyToolkitResolver;
use crate::openhuman::agent::harness::subagent_runner::handoff::ResultHandoffCache;

/// Sub-agent [`ToolSource`]: looks up tools in `extra_tools` then the parent
/// registry, lazily registers toolkit actions the fuzzy filter omitted, rejects
/// names outside the allowlist, and routes execution through the shared
/// [`run_one_tool`] (so sub-agents now get the same approval gate, audit,
/// credential scrub, tokenjuice and timeout as the channel loop), then applies
/// the progressive-disclosure handoff.
pub(super) struct SubagentToolSource<'a> {
    pub(super) parent_tools: &'a [Box<dyn Tool>],
    pub(super) extra_tools: Vec<Box<dyn Tool>>,
    pub(super) allowed_names: HashSet<String>,
    pub(super) lazy_resolver: Option<LazyToolkitResolver>,
    pub(super) advertised_specs: Vec<ToolSpec>,
    pub(super) handoff_cache: Option<&'a ResultHandoffCache>,
    pub(super) policy: crate::openhuman::tools::policy::DefaultToolPolicy,
    pub(super) agent_id: String,
    pub(super) tokenjuice_compression: crate::openhuman::tokenjuice::AgentTokenjuiceCompression,
}

#[async_trait::async_trait]
impl super::super::super::engine::ToolSource for SubagentToolSource<'_> {
    fn request_specs(&self) -> &[ToolSpec] {
        &self.advertised_specs
    }

    async fn execute_call(
        &mut self,
        call: &super::super::super::parse::ParsedToolCall,
        iteration: usize,
        progress: &dyn super::super::super::engine::ProgressReporter,
        progress_call_id: &str,
    ) -> super::super::super::engine::ToolRunResult {
        // Lazy registration: a call for an unknown tool that matches a real
        // action slug in the bound toolkit gets built on the spot and admitted
        // to the allowlist. The fuzzy top-K filter keeps schemas out of the
        // prompt, not out of execution.
        if !self.allowed_names.contains(&call.name) {
            if let Some(resolver) = self.lazy_resolver.as_ref() {
                if let Some(tool) = resolver.resolve(&call.name) {
                    tracing::info!(
                        agent_id = %self.agent_id,
                        tool = %call.name,
                        "[subagent_runner] lazily registered toolkit action outside fuzzy top-K"
                    );
                    self.allowed_names.insert(tool.name().to_string());
                    self.extra_tools.push(tool);
                }
            }
        }

        if !self.allowed_names.contains(&call.name) {
            tracing::warn!(
                agent_id = %self.agent_id,
                tool = %call.name,
                "[subagent_runner] tool not in allowlist for this sub-agent"
            );
            let iteration_u32 = (iteration + 1) as u32;
            // Not-in-allowlist reject path: no resolved tool to label, so defer
            // to the client formatter. (Allowed sub-agent calls run through the
            // shared `run_one_tool`, which computes the label there.)
            progress
                .tool_started(
                    progress_call_id,
                    &call.name,
                    &call.arguments,
                    iteration_u32,
                    None,
                    None,
                )
                .await;
            let mut available: Vec<&str> = self.allowed_names.iter().map(|s| s.as_str()).collect();
            if let Some(resolver) = self.lazy_resolver.as_ref() {
                available.extend(resolver.known_slugs());
            }
            available.sort_unstable();
            available.dedup();
            let text = format!(
                "Error: tool '{}' is not available to the {} sub-agent. Available tools: {}",
                call.name,
                self.agent_id,
                available.join(", ")
            );
            progress
                .tool_completed(progress_call_id, &call.name, false, &text, 0, iteration_u32)
                .await;
            return super::super::super::engine::ToolRunResult {
                text,
                success: false,
            };
        }

        let tool_opt: Option<&dyn Tool> = self
            .extra_tools
            .iter()
            .find(|t| t.name() == call.name)
            .or_else(|| self.parent_tools.iter().find(|t| t.name() == call.name))
            .map(|b| b.as_ref());
        let outcome = super::super::super::engine::run_one_tool(
            tool_opt,
            call,
            iteration,
            progress,
            &self.policy,
            None,
            progress_call_id,
            self.tokenjuice_compression,
        )
        .await;

        let text = match self.handoff_cache {
            Some(cache) => apply_handoff(cache, &call.name, "", &self.agent_id, outcome.text),
            None => outcome.text,
        };
        super::super::super::engine::ToolRunResult {
            text,
            success: outcome.success,
        }
    }
}
