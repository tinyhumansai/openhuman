//! Narrow, KV-cache-stable system-prompt renderer for typed sub-agents.
//!
//! Purpose-built alternative to
//! [`SystemPromptBuilder::for_subagent`](crate::agent::prompts::builder::SystemPromptBuilder::for_subagent)
//! for callers that only hold indices into the parent's tool vec.

use super::super::builder::GLOBAL_STYLE_SUFFIX;
use super::super::sections::GROUNDING_BODY;
use super::super::types::*;
use super::workspace_files::{inject_workspace_file, write_agents_md_blocks};
use std::fmt::Write;
use std::path::Path;
use tinytools_agent::dialect::{CodeDialect, NativeDialect, PFormatDialect, ToolDialect};

// ─────────────────────────────────────────────────────────────────────────────
// Sub-agent prompt renderer
// ─────────────────────────────────────────────────────────────────────────────

/// Render a narrow, KV-cache-stable system prompt for a typed sub-agent.
///
/// This is a purpose-built alternative to
/// [`crate::agent::prompts::builder::SystemPromptBuilder::for_subagent`] for call sites
/// that only have indices into the parent's `&[Box<dyn Tool>]` vec (so they
/// can't cheaply build a filtered owning slice for `ToolsSection`). The
/// output mirrors what `for_subagent` would emit with the matching
/// `omit_*` flags, plus a sub-agent-specific calling-convention
/// preamble and a model-only runtime banner.
///
/// `archetype_body` is the already-loaded archetype markdown — for
/// `PromptSource::Inline` this is the inline string, for
/// `PromptSource::File` this is the file contents loaded by the caller.
/// Callers resolve the source exactly once and hand the body in, so
/// this renderer works uniformly for both definition shapes.
///
/// `options` carries the per-definition rendering flags (safety, etc.)
/// inverted into positive-sense `include_*` form.
/// [`SubagentRenderOptions::narrow`] preserves the historical behaviour.
///
/// # KV cache stability
///
/// The rendered bytes MUST be a pure function of:
/// - the `archetype_body` (archetype role prompt)
/// - the filtered tool set (names, descriptions, schemas)
/// - the workspace directory
/// - the resolved model name
/// - the `options` (all static per definition)
///
/// Anything that varies across invocations at the *same* call site
/// (e.g. `chrono::Local::now()`, hostnames, pids, turn counters) is
/// forbidden here. Repeat spawns of the same sub-agent within a session
/// must produce byte-identical system prompts so the inference
/// backend's automatic prefix caching can reuse the prefill from the
/// previous run. Time-of-day information, if a sub-agent needs it,
/// belongs in the user message — not the system prompt.
pub fn render_subagent_system_prompt(
    workspace_dir: &Path,
    model_name: &str,
    allowed_indices: &[usize],
    parent_tools: &[Box<dyn tinytools::Tool>],
    extra_tools: &[Box<dyn tinytools::Tool>],
    archetype_body: &str,
    options: SubagentRenderOptions,
    tool_call_format: ToolCallFormat,
    connected_integrations: &[ConnectedIntegration],
) -> String {
    render_subagent_system_prompt_with_format(
        workspace_dir,
        model_name,
        allowed_indices,
        parent_tools,
        extra_tools,
        archetype_body,
        options,
        tool_call_format,
        connected_integrations,
        None,
        None,
    )
}

/// Inner renderer that accepts an explicit [`ToolCallFormat`] so callers
/// that know the active dispatcher format can thread it through. The
/// public [`render_subagent_system_prompt`] uses the preferred Python dialect
/// by default.
///
/// `agents_md_global` / `agents_md_local` are the pre-loaded AGENTS.md layers
/// (see [`crate::agent::prompts::agents_md::load_agents_md_layers`]); `None`/`None` (the value
/// the public wrapper passes) renders no AGENTS.md block. When present they are
/// injected as `## Project instructions (AGENTS.md)` right after the identity block
/// and before the tool catalogue — matching the section order of the default /
/// sub-agent builders.
#[allow(clippy::too_many_arguments)]
pub fn render_subagent_system_prompt_with_format(
    workspace_dir: &Path,
    model_name: &str,
    allowed_indices: &[usize],
    parent_tools: &[Box<dyn tinytools::Tool>],
    extra_tools: &[Box<dyn tinytools::Tool>],
    archetype_body: &str,
    options: SubagentRenderOptions,
    tool_call_format: ToolCallFormat,
    _connected_integrations: &[ConnectedIntegration],
    agents_md_global: Option<&str>,
    agents_md_local: Option<&str>,
) -> String {
    let mut out = String::new();

    // 1. Archetype role prompt. Works for `PromptSource::Inline`,
    //    `PromptSource::File`, and `PromptSource::Dynamic` because the
    //    caller preloaded the body via `load_prompt_source`.
    let trimmed = archetype_body.trim();
    if !trimmed.is_empty() {
        out.push_str(trimmed);
        out.push_str("\n\n");
    }

    // 1b. Optional identity block. Off by default; turned on when the
    //     definition sets `omit_identity = false`. Renders the same
    //     OpenClaw bootstrap files the main agent loads, keeping the
    //     byte layout stable across repeat spawns of the same
    //     definition within a session.
    if options.include_identity {
        out.push_str("## Project Context\n\n");
        out.push_str(
            "The following workspace files define your identity, behavior, and context.\n\n",
        );
        for file in &["SOUL.md", "IDENTITY.md"] {
            inject_workspace_file(&mut out, workspace_dir, file);
        }
    }

    // 1c. Project instructions (AGENTS.md), pre-loaded by the caller and shared
    //     with the section-based builders through `write_agents_md_blocks` so
    //     the byte layout can never drift between the two paths. Placed after
    //     the identity block and before the tool catalogue, matching the default
    //     section order. Skipped entirely when both layers are `None`.
    write_agents_md_blocks(&mut out, agents_md_global, agents_md_local);

    // 2. TinyTools owns both the catalogue and calling protocol.  Collect the
    // filtered surface in deterministic order, then delegate its rendering to
    // the same dialect that parses the model response.
    let mut tool_specs = Vec::new();
    for &i in allowed_indices {
        let Some(tool) = parent_tools.get(i) else {
            tracing::warn!(
                index = i,
                tool_count = parent_tools.len(),
                "[context::prompt] dropping out-of-range tool index in subagent render"
            );
            continue;
        };
        tool_specs.push(tinytools::ToolSpec {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            parameters: tool.parameters_schema(),
        });
    }
    tool_specs.extend(extra_tools.iter().map(|tool| tinytools::ToolSpec {
        name: tool.name().to_string(),
        description: tool.description().to_string(),
        parameters: tool.parameters_schema(),
    }));
    if !tool_specs.is_empty() {
        out.push_str(&render_tool_dialect_prompt(tool_call_format, &tool_specs));
    }
    out.push_str("\nUse the provided tools to accomplish the task. Reply with a concise, dense final answer when you have one — the parent agent will weave it back into the user-visible response.\n\n");

    // 3b. Optional safety preamble. Definitions that do work with real
    //     side-effects set `omit_safety_preamble = false`, which the narrow
    //     renderer used to silently drop — we now honour the flag.
    //     Byte-identical to `SafetySection::build`.
    if options.include_safety_preamble {
        out.push_str(
            "## Safety\n\n- Do not exfiltrate private data.\n- Do not run destructive commands without asking.\n- Do not bypass oversight or approval mechanisms.\n- Prefer `trash` over `rm`.\n- When in doubt, ask before acting externally.\n\n",
        );
    }

    // 3b'. Grounding / anti-hallucination contract. Always emitted (like the
    //      static chain): every spawned sub-agent gets the same floor.
    //      Sourced from the shared `GROUNDING_BODY` const so this narrow
    //      renderer can never drift from `GroundingSection`.
    out.push_str(GROUNDING_BODY);
    out.push_str("\n\n");

    // 3c/3d. `## Available Skills` and `## Connected Integrations`
    //        are no longer emitted here. Each agent that needs them
    //        renders its own block in its `prompt.rs`
    //        (orchestrator/welcome own the delegator voice). Legacy Inline/File-sourced TOML agents
    //        that still route through this helper simply don't get
    //        either block — which matches the fact that none of them
    //        currently opt in.

    // 4. Workspace so the model knows where it is. Intentionally stable:
    //    no datetime, no hostname, no pid — see the KV-cache note above.
    let _ = writeln!(
        out,
        "## Workspace\n\nWorking directory: `{}`\n",
        workspace_dir.display()
    );

    // 6. Runtime banner — model name only. Stable for the lifetime of
    //    this sub-agent's definition.
    let _ = writeln!(out, "## Runtime\n\nModel: {model_name}");
    out.push('\n');
    out.push_str(GLOBAL_STYLE_SUFFIX);

    out
}

/// Ask TinyTools to produce the exact catalogue and protocol that its
/// corresponding dialect accepts. OpenHuman deliberately owns no syntax here.
fn render_tool_dialect_prompt(format: ToolCallFormat, tools: &[tinytools::ToolSpec]) -> String {
    match format {
        // Native providers receive the catalogue in the request's structured
        // `tools` field. Rendering it in the system prompt duplicates large
        // integration toolkits and can exhaust the context window.
        ToolCallFormat::Native => NativeDialect.prompt_instructions(&[]),
        ToolCallFormat::Json => harness_json_tool_prompt(tools),
        ToolCallFormat::PFormat => {
            let registry = tinytools_agent::build_registry(
                tools
                    .iter()
                    .map(|tool| (tool.name.as_str(), &tool.parameters)),
            );
            format!(
                "{}\n{}",
                tinytools_agent::dialect::render_pformat_catalogue(tools),
                PFormatDialect::new(registry).prompt_instructions(tools)
            )
        }
        ToolCallFormat::Python | ToolCallFormat::TypeScript => {
            let style = format.code_style().expect("code format has a code style");
            format!(
                "{}\n{}",
                tinytools_agent::render::render_code_catalogue(tools, style),
                CodeDialect::instructions(style)
            )
        }
    }
}

/// Use TinyAgents' complete text-mode prompt contract rather than teaching a
/// local JSON convention that could drift from transcript replay and parsing.
pub(crate) fn harness_json_tool_prompt(tools: &[tinytools::ToolSpec]) -> String {
    let schemas: Vec<tinyinference_llm::tool::ToolSchema> = tools
        .iter()
        .map(|tool| {
            tinyinference_llm::tool::ToolSchema::new(
                tool.name.clone(),
                tool.description.clone(),
                tool.parameters.clone(),
            )
        })
        .collect();
    tinyagents_harness::tool::prompt_tool_instructions(&schemas)
}
