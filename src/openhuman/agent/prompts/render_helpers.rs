//! Free `render_*` functions, sub-agent prompt renderer, and workspace-file
//! I/O helpers.
//!
//! The `render_*` family provides a functional interface over the section
//! structs in [`super::sections`] — `agents/<id>/prompt.rs` builders call
//! these to assemble their own final system prompt without needing the full
//! [`super::builder::SystemPromptBuilder`] machinery.

use super::builder::GLOBAL_STYLE_SUFFIX;
use super::sections::*;
use super::types::*;
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::fmt::Write;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::OnceLock;

// ─────────────────────────────────────────────────────────────────────────────
// Section render helpers (functional wrappers over section structs)
// ─────────────────────────────────────────────────────────────────────────────

/// Render the `## Project Context` identity block
/// (`SOUL.md` / `IDENTITY.md` / optionally `HEARTBEAT.md`).
pub fn render_identity(ctx: &PromptContext<'_>) -> Result<String> {
    IdentitySection.build(ctx)
}

/// Render the `PROFILE.md` + `MEMORY.md` user-file injection.
/// Empty when neither `ctx.include_profile` nor `ctx.include_memory_md`
/// is set.
pub fn render_user_files(ctx: &PromptContext<'_>) -> Result<String> {
    UserFilesSection.build(ctx)
}

/// Render the tree-summariser user-memory block.
pub fn render_user_memory(ctx: &PromptContext<'_>) -> Result<String> {
    UserMemorySection.build(ctx)
}

/// Render the privileged `## User Reflections` block. Empty when the
/// learning subsystem has not captured any reflections yet.
pub fn render_user_reflections(ctx: &PromptContext<'_>) -> Result<String> {
    UserReflectionsSection.build(ctx)
}

/// Render the `## Tools` catalogue in the dispatcher's tool-call format.
pub fn render_tools(ctx: &PromptContext<'_>) -> Result<String> {
    ToolsSection.build(ctx)
}

/// Render the static `## Safety` block.
pub fn render_safety() -> String {
    SafetySection
        .build(&empty_prompt_context_for_static_sections())
        .expect("SafetySection::build is infallible")
}

/// Render the canonical grounding / anti-hallucination contract
/// ([`GROUNDING_BODY`]). Dynamic `agents/<id>/prompt.rs` builders call this
/// so they inherit the exact same anti-fabrication floor as the static
/// section chain — single source of truth, no drift.
pub fn render_grounding() -> &'static str {
    GROUNDING_BODY
}

// `render_skills` and `render_connected_integrations` helpers are
// gone — `## Available Skills` lives in `integrations_agent/prompt.rs`, and
// the connected-integrations / delegation-guide blocks each live in
// their owning agent's `prompt.rs` so no branching-on-agent-id logic
// needs to exist here.

/// Render the `## Workspace` block (working directory + file listing
/// bounds) — part of the dynamic, per-request suffix.
pub fn render_workspace(ctx: &PromptContext<'_>) -> Result<String> {
    WorkspaceSection.build(ctx)
}

/// Render the `## Runtime` block (model name, dispatcher format) —
/// dynamic.
pub fn render_runtime(ctx: &PromptContext<'_>) -> Result<String> {
    RuntimeSection.build(ctx)
}

/// Render the `## Current Date & Time` block: the static time-discipline
/// *rules* (greeting/clock grounding + the gated `resolve_time` rule). The
/// concrete "now" is **not** here — it rides the user message per turn via
/// [`current_datetime_line`] so it stays fresh and keeps this section
/// byte-stable for prefix caching (#3602).
pub fn render_datetime(ctx: &PromptContext<'_>) -> Result<String> {
    DateTimeSection.build(ctx)
}

/// Canonical one-line "now" stamp, injected per turn alongside the user
/// message by both the main session loop (`session::turn`) and the
/// sub-agent runner so every flow reports the current time identically
/// (#3602). Local time + IANA zone + `%Z`/offset + weekday, so the model
/// can localize greetings and date math without a tool call.
///
/// Deliberately lives on the *user message*, never the cached
/// system-prompt prefix: `Local::now()` is volatile, so freezing it into
/// the prefix both busts the KV cache and goes stale across a long-lived
/// session. The static grounding *rule* that tells the model to read this
/// line lives in [`DateTimeSection`] / [`render_datetime`].
pub fn current_datetime_line() -> String {
    // When the host resolves an IANA zone, stamp local time + that zone. When
    // it can't (CI, stripped containers), fall back to true UTC — formatting
    // `Utc::now()` so the time, offset, and zone label all agree rather than
    // pairing a "UTC" label with a local clock/offset.
    match iana_time_zone::get_timezone() {
        Ok(iana) => {
            let now = chrono::Local::now();
            format!(
                "Current Date & Time: {} {} ({}, UTC{}), {}",
                now.format("%Y-%m-%d %H:%M:%S"),
                iana,
                now.format("%Z"),
                now.format("%:z"),
                now.format("%A"),
            )
        }
        Err(_) => {
            let now = chrono::Utc::now();
            format!(
                "Current Date & Time: {} UTC (UTC, UTC+00:00), {}",
                now.format("%Y-%m-%d %H:%M:%S"),
                now.format("%A"),
            )
        }
    }
}

/// Render the `## User` identity block. Empty when
/// [`PromptContext::user_identity`] is unset or has no populated
/// fields. See issue #926.
pub fn render_user_identity(ctx: &PromptContext<'_>) -> Result<String> {
    UserIdentitySection.build(ctx)
}

/// Compose the full ambient-environment block — runtime + user
/// identity + current date/time, in that order.
///
/// Per-agent `prompt.rs` builders call this once near the end of their
/// assembly so every agent reports the same machine-readable view of
/// "where am I, who is the user, what time is it" (issue #926).
/// Datetime is appended last so the time-volatile section sits at the
/// tail of the prompt and the rest of the prefix stays cache-stable
/// across turns within the same minute, matching the convention used
/// by [`super::builder::SystemPromptBuilder::with_defaults`].
pub fn render_ambient_environment(ctx: &PromptContext<'_>) -> Result<String> {
    let mut out = String::with_capacity(512);
    let runtime = render_runtime(ctx)?;
    if !runtime.trim().is_empty() {
        out.push_str(runtime.trim_end());
        out.push_str("\n\n");
    }
    let user = render_user_identity(ctx)?;
    if !user.trim().is_empty() {
        out.push_str(user.trim_end());
        out.push_str("\n\n");
    }
    let datetime = render_datetime(ctx)?;
    if !datetime.trim().is_empty() {
        out.push_str(datetime.trim_end());
        out.push('\n');
    }
    Ok(out)
}

/// Format a memory item's `updated_at` as an absolute UTC date label
/// for prompt injection, e.g. `2026-05-25`.
///
/// Absolute (not relative "N days ago") on purpose: memory sections sit
/// near the front of the KV-cache-stable system prompt, so a label that
/// changes daily would bust the cached prefix for everything after it.
/// An absolute date only changes when the underlying memory does. The
/// model judges staleness by comparing this against the injected current
/// date. Shared by [`UserMemorySection`] and the working-memory block in
/// `agent::memory_loader`. (#2944)
pub fn memory_date_label(updated_at: DateTime<Utc>) -> String {
    updated_at.format("%Y-%m-%d").to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
// Sub-agent prompt renderer
// ─────────────────────────────────────────────────────────────────────────────

/// Render a narrow, KV-cache-stable system prompt for a typed sub-agent.
///
/// This is a purpose-built alternative to
/// [`super::builder::SystemPromptBuilder::for_subagent`] for call sites
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
    parent_tools: &[Box<dyn crate::openhuman::tools::Tool>],
    extra_tools: &[Box<dyn crate::openhuman::tools::Tool>],
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
    parent_tools: &[Box<dyn crate::openhuman::tools::Tool>],
    extra_tools: &[Box<dyn crate::openhuman::tools::Tool>],
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
        let render_one =
            |out: &mut String, tool: &dyn crate::openhuman::tools::Tool| match tool_call_format {
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

    // 3b'. Grounding / anti-hallucination contract. Always emitted (like the
    //      static chain): every spawned sub-agent gets the same floor.
    //      Sourced from the shared `GROUNDING_BODY` const so this narrow
    //      renderer can never drift from `GroundingSection`.
    out.push_str(GROUNDING_BODY);
    out.push_str("\n\n");

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
    out.push('\n');
    out.push_str(GLOBAL_STYLE_SUFFIX);

    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Workspace-file I/O helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Ensure the workspace file is up-to-date with the compiled-in default.
///
/// On first install the file doesn't exist → write it. On subsequent runs
/// we store a hash of the compiled-in content in a sidecar file
/// (`.{filename}.builtin-hash`). If the hash changes (code was updated),
/// the disk file is overwritten so prompt improvements ship automatically.
/// User edits between code releases are preserved — we only overwrite when
/// the built-in default itself changes.
pub fn sync_workspace_file(workspace_dir: &Path, filename: &str) {
    let default_content = default_workspace_file_content(filename);
    if default_content.is_empty() {
        return;
    }

    let path = workspace_dir.join(filename);
    let hash_path = workspace_dir.join(format!(".{filename}.builtin-hash"));

    // Compute a simple hash of the current compiled-in content.
    let current_hash = {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        default_content.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    };

    // Read the last-written hash (if any).
    let stored_hash = std::fs::read_to_string(&hash_path).unwrap_or_default();
    let stored_hash = stored_hash.trim();

    if stored_hash == current_hash && path.exists() {
        // Built-in hasn't changed and file exists — nothing to do.
        return;
    }

    // Decide whether to overwrite the existing file. Two safe cases:
    //   1. File doesn't exist yet — first install, write the default.
    //   2. File exists AND its current hash matches the stored builtin
    //      hash — the user hasn't edited it since we last wrote it, so
    //      it's safe to ship the new default.
    // Otherwise the file has been hand-edited between releases; leave
    // the user's version in place and just update the stored hash so we
    // stop re-comparing against the old default on every boot.
    let file_exists = path.exists();
    let user_unmodified = if file_exists {
        match std::fs::read_to_string(&path) {
            Ok(disk) => {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                disk.hash(&mut hasher);
                let disk_hash = format!("{:016x}", hasher.finish());
                disk_hash == stored_hash
            }
            Err(_) => false,
        }
    } else {
        false
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if !file_exists || user_unmodified {
        if let Err(e) = std::fs::write(&path, default_content) {
            log::warn!("[agent:prompt] failed to write workspace file {filename}: {e}");
            return;
        }
        log::info!("[agent:prompt] updated workspace file {filename} (builtin content changed)");
    } else {
        log::info!(
            "[agent:prompt] keeping user-edited workspace file {filename} (builtin changed but disk contents diverge)"
        );
    }
    let _ = std::fs::write(&hash_path, &current_hash);
}

/// Inject `filename` from `workspace_dir` into `prompt`, truncated to
/// [`BOOTSTRAP_MAX_CHARS`]. Thin wrapper around
/// [`inject_workspace_file_capped`] for bootstrap-class files
/// (`SOUL.md`, `IDENTITY.md`, `HEARTBEAT.md`).
pub fn inject_workspace_file(prompt: &mut String, workspace_dir: &Path, filename: &str) {
    inject_workspace_file_capped(prompt, workspace_dir, filename, BOOTSTRAP_MAX_CHARS);
}

/// Inject pre-loaded string content into `prompt` under a `### label` heading,
/// capped at `max_chars`. Mirrors the format of [`inject_snapshot_content`]
/// and [`inject_workspace_file_capped`] but takes a `&str` instead of a file
/// path. Used for personality-specific overrides (`personality_soul_md`,
/// `personality_memory_md`) on [`PromptContext`] so a swap from the file-based
/// loader to an inline override is byte-compatible with the workspace-file path.
///
/// Empty/whitespace content is silently skipped.
pub fn inject_inline_content(prompt: &mut String, label: &str, content: &str, max_chars: usize) {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return;
    }
    let _ = writeln!(prompt, "### {label}\n");
    let truncated = if trimmed.chars().count() > max_chars {
        trimmed
            .char_indices()
            .nth(max_chars)
            .map(|(idx, _)| &trimmed[..idx])
            .unwrap_or(trimmed)
    } else {
        trimmed
    };
    prompt.push_str(truncated);
    if truncated.len() < trimmed.len() {
        let _ = writeln!(
            prompt,
            "\n\n[... truncated at {max_chars} chars — use `read` for full file]\n"
        );
    } else {
        prompt.push_str("\n\n");
    }
}

/// for the output header and truncation semantics.
///
/// Empty/whitespace content is silently skipped, mirroring the file
/// loader's "no noisy placeholder" behaviour.
pub fn inject_snapshot_content(prompt: &mut String, label: &str, content: &str, max_chars: usize) {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return;
    }
    let _ = writeln!(prompt, "### {label}\n");
    let truncated = if trimmed.chars().count() > max_chars {
        trimmed
            .char_indices()
            .nth(max_chars)
            .map(|(idx, _)| &trimmed[..idx])
            .unwrap_or(trimmed)
    } else {
        trimmed
    };
    prompt.push_str(truncated);
    if truncated.len() < trimmed.len() {
        let _ = writeln!(
            prompt,
            "\n\n[... truncated at {max_chars} chars — use `read` for full file]\n"
        );
    } else {
        prompt.push_str("\n\n");
    }
}

/// Inject `filename` into `prompt` with an explicit character budget.
///
/// Used directly by callers that want a tighter cap than
/// [`BOOTSTRAP_MAX_CHARS`] — notably `PROFILE.md` and `MEMORY.md` which
/// are user-specific, potentially growing, and do not warrant a full
/// 20K-char budget (see [`USER_FILE_MAX_CHARS`]).
///
/// Missing / empty files are silently skipped so callers can inject
/// optional files unconditionally without emitting a noisy placeholder.
///
/// **KV-cache contract:** the output is a pure function of `filename`,
/// file bytes at call time, and `max_chars`. Callers must invoke this
/// once per session — re-reading mid-session breaks the inference
/// backend's automatic prefix cache. See the byte-stability note on
/// [`render_subagent_system_prompt`].
pub fn inject_workspace_file_capped(
    prompt: &mut String,
    workspace_dir: &Path,
    filename: &str,
    max_chars: usize,
) {
    let path = workspace_dir.join(filename);

    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                return;
            }
            let _ = writeln!(prompt, "### {filename}\n");
            let truncated = if trimmed.chars().count() > max_chars {
                trimmed
                    .char_indices()
                    .nth(max_chars)
                    .map(|(idx, _)| &trimmed[..idx])
                    .unwrap_or(trimmed)
            } else {
                trimmed
            };
            prompt.push_str(truncated);
            if truncated.len() < trimmed.len() {
                let _ = writeln!(
                    prompt,
                    "\n\n[... truncated at {max_chars} chars — use `read` for full file]\n"
                );
            } else {
                prompt.push_str("\n\n");
            }
        }
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => {
                // Keep prompt focused: missing optional identity/bootstrap files should not
                // add noisy placeholders that dilute tool-calling instructions.
            }
            _ => {
                log::debug!("[prompt] failed to read {}: {e}", path.display());
            }
        },
    }
}

pub fn default_workspace_file_content(filename: &str) -> &'static str {
    // The bundled identity files live at `src/openhuman/agent/prompts/`
    // (owned by the `agent/` tree because they describe agent identity).
    // This module is under `src/openhuman/context/`, so the relative path
    // walks up one level and back into `agent/prompts/`.
    match filename {
        "SOUL.md" => include_str!("SOUL.md"),
        "IDENTITY.md" => include_str!("IDENTITY.md"),
        "HEARTBEAT.md" => {
            "# Periodic Tasks\n\n# Add tasks below (one per line, starting with `- `)\n"
        }
        // The agent's long-term goals list. Header-only template so the file
        // is discoverable in the workspace from first boot; items are managed
        // by the memory_goals domain (RPC / tools / enrichment agent). Must
        // match `GoalsDoc::render()` of an empty doc so parse↔render is stable.
        "MEMORY_GOALS.md" => "# Long-term Goals\n\n",
        _ => "",
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Build a throwaway `PromptContext` for sections whose `build` only
/// uses static/immutable inputs (currently just `SafetySection`). Keeps
/// the `render_safety()` free function from forcing callers to
/// manufacture a full context when they only need the static text.
fn empty_prompt_context_for_static_sections() -> PromptContext<'static> {
    static EMPTY_TOOLS: &[PromptTool<'static>] = &[];
    static EMPTY_WORKFLOWS: &[crate::openhuman::workflows::Workflow] = &[];
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
        workflows: EMPTY_WORKFLOWS,
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: EMPTY_INTEGRATIONS,
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
    }
}

/// Build a P-Format signature line (`name[a|b|c]`) from a `&dyn Tool`.
/// Used by `render_subagent_system_prompt` which operates on `Box<dyn Tool>`
/// directly (no intermediate `PromptTool`). Mirrors the `PromptTool` variant
/// below — both BTreeMap-iterate the schema's `properties` in the same order.
fn render_pformat_signature_for_box_tool(tool: &dyn crate::openhuman::tools::Tool) -> String {
    let schema = tool.parameters_schema();
    let names: Vec<String> = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    if names.is_empty() {
        format!("{}[]", tool.name())
    } else {
        format!("{}[{}]", tool.name(), names.join("|"))
    }
}
