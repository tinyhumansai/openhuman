use crate::openhuman::agent::prompts::sections::render_pformat_signature_for_box_tool;
use crate::openhuman::agent::prompts::types::{
    ConnectedIntegration, LearnedContextData, PromptContext, PromptTool, SubagentRenderOptions,
    ToolCallFormat,
};
use crate::openhuman::agent::prompts::workspace_files::{
    inject_workspace_file, inject_workspace_file_capped,
};
use crate::openhuman::skills::Skill;
use crate::openhuman::tools::Tool;
use std::fmt::Write;
use std::path::Path;
use std::sync::OnceLock;

use crate::openhuman::agent::prompts::types::USER_FILE_MAX_CHARS;

/// Render a narrow, KV-cache-stable system prompt for a typed sub-agent.
///
/// This is a purpose-built alternative to
/// [`SystemPromptBuilder::for_subagent`] for call sites that only have
/// indices into the parent's `&[Box<dyn Tool>]` vec (so they can't
/// cheaply build a filtered owning slice for `ToolsSection`). The
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
    parent_tools: &[Box<dyn Tool>],
    extra_tools: &[Box<dyn Tool>],
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
    )
}

/// Inner renderer that accepts an explicit [`ToolCallFormat`] so callers
/// that know the active dispatcher format can thread it through. The
/// public [`render_subagent_system_prompt`] defaults to PFormat for
/// backwards compatibility.
pub fn render_subagent_system_prompt_with_format(
    workspace_dir: &Path,
    model_name: &str,
    allowed_indices: &[usize],
    parent_tools: &[Box<dyn Tool>],
    extra_tools: &[Box<dyn Tool>],
    archetype_body: &str,
    options: SubagentRenderOptions,
    tool_call_format: ToolCallFormat,
    _connected_integrations: &[ConnectedIntegration],
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

    // 1c. PROFILE.md (onboarding enrichment output) and MEMORY.md
    //     (archivist-curated long-term memory). Each is gated on its own
    //     flag and capped at `USER_FILE_MAX_CHARS` (~1000 tokens) so a
    //     growing on-disk file can't push the system prompt out of the
    //     cache-friendly prefix range.
    //
    //     KV-cache contract: once these files land in a session's
    //     rendered prompt the bytes are frozen for the remainder of that
    //     session. Do not re-read them mid-turn — a byte change breaks
    //     the backend's automatic prefix cache. Mid-session writes to
    //     either file are intentionally only visible on the NEXT session.
    if options.include_profile {
        inject_workspace_file_capped(&mut out, workspace_dir, "PROFILE.md", USER_FILE_MAX_CHARS);
    }
    if options.include_memory_md {
        inject_workspace_file_capped(&mut out, workspace_dir, "MEMORY.md", USER_FILE_MAX_CHARS);
    }

    // 2. Filtered tool catalogue. Indices are taken in ascending order
    //    from `allowed_indices`, which itself preserves `parent_tools`
    //    order, so the rendering is deterministic. We use `.get(i)`
    //    defensively even though the current caller (subagent_runner)
    //    only produces in-range indices — a future caller that derives
    //    indices from a different source must not be able to panic this
    //    renderer with a stale index.
    //
    //    Rendering uses the caller-specified `tool_call_format` so
    //    sub-agents and the main dispatcher stay in lockstep.
    // Tool catalogue rendering is dispatcher-format-aware:
    //
    // - **Native**: The provider receives full tool schemas through
    //   the request body's `tools` field (via `filtered_specs` in the
    //   sub-agent runner) and emits structured `tool_calls`. Listing
    //   the same tools again as prose in the system prompt is pure
    //   duplication — for a integrations_agent spawn with 62 dynamic gmail
    //   tools, that duplication added ~54k tokens and blew past the
    //   model's context window. We skip the prose `## Tools` section
    //   entirely in this mode.
    //
    // - **PFormat / Json**: Both are prompt-driven formats — the
    //   model discovers tools by reading the prose `## Tools` section
    //   and emits text-wrapped tool calls (`<tool_call>name[a|b]</tool_call>`
    //   for PFormat, `<tool_call>{"name":...}</tool_call>` for Json).
    //   Neither uses the native `tools` request field, so we MUST
    //   list each tool in prose — including dynamically-registered
    //   `extra_tools` — or the model has no way to know they exist.
    if !matches!(tool_call_format, ToolCallFormat::Native) {
        out.push_str("## Tools\n\n");
        let render_one = |out: &mut String, tool: &dyn Tool| match tool_call_format {
            ToolCallFormat::PFormat => {
                let sig = render_pformat_signature_for_box_tool(tool);
                let _ = writeln!(
                    out,
                    "- **{}**: {}\n  Call as: `{}`",
                    tool.name(),
                    tool.description(),
                    sig
                );
            }
            ToolCallFormat::Json => {
                let _ = writeln!(
                    out,
                    "- **{}**: {}\n  Parameters: `{}`",
                    tool.name(),
                    tool.description(),
                    tool.parameters_schema()
                );
            }
            ToolCallFormat::Native => {
                // Unreachable — outer guard skips Native entirely.
            }
        };
        for &i in allowed_indices {
            let Some(tool) = parent_tools.get(i) else {
                tracing::warn!(
                    index = i,
                    tool_count = parent_tools.len(),
                    "[context::prompt] dropping out-of-range tool index in subagent render"
                );
                continue;
            };
            render_one(&mut out, tool.as_ref());
        }
        for tool in extra_tools {
            render_one(&mut out, tool.as_ref());
        }
    }

    // 3. Sub-agent calling-convention preamble — format-aware.
    //    Sub-agents need the same call format the main dispatcher expects
    //    so their output parses correctly.
    out.push('\n');
    match tool_call_format {
        ToolCallFormat::PFormat => {
            out.push_str(
                "## Tool Use Protocol\n\n\
                 Tool calls use **P-Format**: compact, positional, pipe-delimited syntax \
                 wrapped in `<tool_call>` tags.\n\n\
                 ```\n<tool_call>\ntool_name[arg1|arg2]\n</tool_call>\n```\n\n\
                 Arguments are positional — match the order shown in each tool's `Call as:` \
                 signature above (alphabetical by parameter name). \
                 Escape `|` as `\\|`, `]` as `\\]` inside values. \
                 You may emit multiple `<tool_call>` blocks per response.\n\n\
                 Use the provided tools to accomplish the task. Reply with a concise, dense \
                 final answer when you have one — the parent agent will weave it back into the \
                 user-visible response.\n\n",
            );
        }
        ToolCallFormat::Json => {
            out.push_str(
                "## Tool Use Protocol\n\n\
                 To use a tool, wrap a JSON object in `<tool_call></tool_call>` tags:\n\n\
                 ```\n<tool_call>\n{\"name\": \"tool_name\", \"arguments\": {\"param\": \"value\"}}\n</tool_call>\n```\n\n\
                 You may emit multiple `<tool_call>` blocks in a single response.\n\n\
                 Use the provided tools to accomplish the task. Reply with a concise, dense \
                 final answer when you have one — the parent agent will weave it back into the \
                 user-visible response.\n\n",
            );
        }
        ToolCallFormat::Native => {
            out.push_str(
                "Use the provided tools via the model's native tool-calling output. \
                 Reply with a concise, dense final answer when you have one — the parent \
                 agent will weave it back into the user-visible response.\n\n",
            );
        }
    }

    // 3b. Optional safety preamble. Definitions that do work with real
    //     side-effects (code_executor, tool_maker, integrations_agent) set
    //     `omit_safety_preamble = false` so the narrow renderer used to
    //     silently drop that instruction — we now honour the flag.
    //     Byte-identical to `SafetySection::build`.
    if options.include_safety_preamble {
        out.push_str(
            "## Safety\n\n- Do not exfiltrate private data.\n- Do not run destructive commands without asking.\n- Do not bypass oversight or approval mechanisms.\n- Prefer `trash` over `rm`.\n- When in doubt, ask before acting externally.\n\n",
        );
    }

    // 3c/3d. `## Available Skills` and `## Connected Integrations`
    //        are no longer emitted here. Each agent that needs them
    //        renders its own block in its `prompt.rs` (integrations_agent
    //        owns the executor voice, orchestrator/welcome own the
    //        delegator voice). Legacy Inline/File-sourced TOML agents
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

    out
}

/// Build a throwaway `PromptContext` for sections whose `build` only
/// uses static/immutable inputs (currently just `SafetySection`). Keeps
/// the `render_safety()` free function from forcing callers to
/// manufacture a full context when they only need the static text.
pub(super) fn empty_prompt_context_for_static_sections() -> PromptContext<'static> {
    static EMPTY_TOOLS: &[PromptTool<'static>] = &[];
    static EMPTY_SKILLS: &[Skill] = &[];
    static EMPTY_INTEGRATIONS: &[ConnectedIntegration] = &[];
    // SAFETY: the &HashSet reference must outlive the returned context;
    // a leaked OnceLock-style allocation gives us a permanent 'static
    // anchor without adding runtime cost on the hot path.
    static EMPTY_VISIBLE: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
    let visible = EMPTY_VISIBLE.get_or_init(std::collections::HashSet::new);
    PromptContext {
        workspace_dir: std::path::Path::new(""),
        model_name: "",
        agent_id: "",
        tools: EMPTY_TOOLS,
        skills: EMPTY_SKILLS,
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: EMPTY_INTEGRATIONS,
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
    }
}
