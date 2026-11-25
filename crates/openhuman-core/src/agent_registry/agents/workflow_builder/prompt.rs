//! System prompt builder for the `workflow_builder` built-in agent (Phase 5a).
//!
//! Composes the module-owned archetype text with OpenHuman's scoped runtime
//! sections. The caller fetches the archetype through TinyFlows Copilot before
//! invoking this synchronous builder.

use crate::agent::prompts::PromptContext;
use anyhow::{anyhow, Result};

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    let archetype = ctx.flow_archetype_text.ok_or_else(|| {
        anyhow!("workflow_builder requires a scoped TinyFlows GetPrompt response")
    })?;
    let mut prompt = super::super::flow_prompts::build_with_archetype(ctx, archetype)?;
    prompt.push_str(GRAPH_HANDLE_GUIDANCE);
    Ok(prompt)
}

/// A whole graph re-emitted as tool-call JSON is where small models break the
/// JSON, and a call with broken arguments never runs: live, a builder resent
/// the same malformed inline graph to `validate_workflow` until the failure
/// breaker halted it, while a run that iterated on a draft passed.
const GRAPH_HANDLE_GUIDANCE: &str = "\n\n## Passing graphs to tools\n\n\
- Whenever a `draft_id` or `flow_id` exists, pass it to `validate_workflow` and \
`dry_run_workflow` instead of the graph, and change the graph with `edit_workflow` \
ops on that id instead of re-emitting it.\n\
- With no id yet, an inline `graph` is the only source: keep it to the nodes and \
config fields the step needs.\n\
- If a call is answered that its arguments are not valid JSON, do not resend the \
same graph. Rebuild it smaller (fewer nodes per call, shorter config text) and \
check that before adding the rest.\n";
