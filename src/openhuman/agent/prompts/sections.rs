//! Concrete [`PromptSection`] implementations.
//!
//! Each unit struct renders one logical block of the system prompt.
//! The rendering logic delegates to the free helpers in
//! [`super::render_helpers`] for workspace-file injection and
//! sub-agent plumbing.

use super::render_helpers::{
    inject_inline_content, inject_snapshot_content, inject_workspace_file,
    inject_workspace_file_capped, sync_workspace_file,
};
use super::types::*;
use anyhow::Result;
use std::fmt::Write;

// ─────────────────────────────────────────────────────────────────────────────
// Special sections (archetype, dynamic, reflection)
// ─────────────────────────────────────────────────────────────────────────────

/// "Memory context" section for chat threads spawned from a subconscious
/// reflection (#623). Renders the resolved [`SourceChunk`]s that the
/// subconscious LLM cited when it produced the reflection — gives the
/// orchestrator the same memory context the reflection-LLM had, so the
/// user can drill into the observation without the orchestrator
/// hallucinating details it never saw.
///
/// Chunks are passed in at construction (snapshot at session-start) so
/// the rendered bytes stay stable for the whole session, matching the
/// "frozen prompt for prefix cache" contract documented on
/// [`super::builder::SystemPromptBuilder::build`].
pub struct ReflectionMemoryContextSection {
    chunks: Vec<crate::openhuman::subconscious::SourceChunk>,
}

impl ReflectionMemoryContextSection {
    pub fn new(chunks: Vec<crate::openhuman::subconscious::SourceChunk>) -> Self {
        Self { chunks }
    }
}

impl PromptSection for ReflectionMemoryContextSection {
    fn name(&self) -> &str {
        "reflection_memory_context"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        // Skip chunks the resolver couldn't populate — `not_found`,
        // `db_error`, or stub kinds without a wired resolver yet. Earlier
        // versions emitted "(content not yet resolved)" as a placeholder,
        // but the orchestrator picks up that literal string as part of
        // its memory context and ends up echoing it back to the user
        // mid-reply. Better to give the LLM no chunk than a placeholder
        // it'll quote.
        let usable: Vec<&crate::openhuman::subconscious::SourceChunk> = self
            .chunks
            .iter()
            .filter(|c| !c.content.trim().is_empty())
            .collect();
        if usable.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::from("## Memory context\n\n");
        out.push_str(
            "This thread was spawned from a subconscious reflection. The chunks below \
             are what OpenHuman was looking at when it surfaced the observation — \
             use them to ground follow-up answers in the same evidence the reflection \
             was based on.\n\n",
        );
        for chunk in usable {
            let body = chunk.content.replace('\n', " ").trim().to_string();
            let _ = writeln!(
                out,
                "- **{kind}** `{ref_id}`: {body}",
                kind = chunk.kind,
                ref_id = chunk.ref_id,
                body = body,
            );
        }
        Ok(out)
    }
}

/// Sub-agent role prompt — pre-loaded text from an
/// [`crate::openhuman::agent::harness::definition::AgentDefinition`]'s
/// `system_prompt` field. Always rendered first when present.
pub struct ArchetypePromptSection {
    body: String,
}

impl ArchetypePromptSection {
    pub fn new(body: String) -> Self {
        Self { body }
    }
}

impl PromptSection for ArchetypePromptSection {
    fn name(&self) -> &str {
        "archetype_prompt"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        if self.body.trim().is_empty() {
            return Ok(String::new());
        }
        Ok(self.body.clone())
    }
}

/// Section that defers to a [`crate::openhuman::agent::harness::definition::PromptBuilder`]
/// every time it renders, so dynamic prompts (orchestrator, welcome,
/// integrations_agent, …) get to see the live runtime
/// [`PromptContext`] — including `connected_integrations`, which are
/// fetched asynchronously after the builder itself has been
/// constructed.
pub struct DynamicPromptSection {
    builder: crate::openhuman::agent::harness::definition::PromptBuilder,
}

impl DynamicPromptSection {
    pub fn new(builder: crate::openhuman::agent::harness::definition::PromptBuilder) -> Self {
        Self { builder }
    }
}

impl PromptSection for DynamicPromptSection {
    fn name(&self) -> &str {
        "dynamic_prompt"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        (self.builder)(ctx)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Standard section unit structs
// ─────────────────────────────────────────────────────────────────────────────

pub struct IdentitySection;
pub struct ToolsSection;
pub struct SafetySection;
/// Renders the canonical grounding / anti-hallucination contract
/// ([`GROUNDING_BODY`]). Always included; never gated.
pub struct GroundingSection;
// `WorkflowsSection` and `ConnectedIntegrationsSection` previously lived
// here and branched on `ctx.agent_id` to pick between the skill-
// executor and delegator voice. They've been removed — each agent's
// `prompt.rs` now renders its own block inline (integrations_agent owns the
// `## Available Skills` + executor-voice `## Connected Integrations`
// blocks, orchestrator owns `## Delegation Guide — Integrations`,
// welcome owns its onboarding-flavoured connected list).
pub struct WorkspaceSection;
pub struct RuntimeSection;
pub struct DateTimeSection;
pub struct UserMemorySection;
/// Renders explicit user reflections — a privileged memory class
/// distinct from generic tree summaries. Rendered above
/// [`UserMemorySection`] so the orchestrator sees the user's own
/// intentional self-statements before any broader summary block.
///
/// Empty (and skipped) when [`LearnedContextData::reflections`] is
/// empty — keeps the prompt clean for users who haven't yet expressed
/// any reflection-style content.
pub struct UserReflectionsSection;
/// Renders the authenticated user's non-secret identity fields
/// (`id` / `name` / `email`) into the system prompt — see issue #926.
///
/// Empty when [`PromptContext::user_identity`] is `None` or the
/// identity has no populated fields. Tokens, refresh tokens, and any
/// opaque credential material are forbidden — only the three
/// identifying fields ship.
pub struct UserIdentitySection;

/// Injects the user-specific, session-frozen workspace files
/// (`PROFILE.md` + `MEMORY.md`), each capped at [`USER_FILE_MAX_CHARS`].
///
/// Separate from [`IdentitySection`] so agents that strip the project-
/// context preamble (`omit_identity = true` — welcome, orchestrator,
/// the trigger pair) still get their user-file injection at runtime via
/// [`super::builder::SystemPromptBuilder::for_subagent`], which skips
/// `IdentitySection` entirely when `omit_identity` is on.
///
/// Cache-stability: static per session — the whole point of the
/// 2000-char cap and the load-once rule documented on
/// [`AgentDefinition::omit_profile`] / `omit_memory_md`.
pub struct UserFilesSection;

/// Renders the personality roster for the master agent's system prompt.
///
/// When [`PromptContext::personality_roster`] is non-empty, emits an
/// `## Available Personalities` section listing each non-self personality
/// with its `id`, `name`, `description`, and an optional truncated
/// `memory_summary`. Empty (and skipped) for non-master agents.
pub struct PersonalityRosterSection;

// ─────────────────────────────────────────────────────────────────────────────
// PromptSection implementations
// ─────────────────────────────────────────────────────────────────────────────

impl PromptSection for PersonalityRosterSection {
    fn name(&self) -> &str {
        "personality_roster"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        if ctx.personality_roster.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::from("## Available Personalities\n\n");
        out.push_str(
            "You are the master agent. You can delegate tasks to these personality agents \
             using the `delegate_to_personality` tool. Each personality has its own memory, \
             identity, and expertise.\n\n",
        );
        for entry in &ctx.personality_roster {
            out.push_str(&format!(
                "- **{}** (`{}`): {}",
                entry.name, entry.id, entry.description
            ));
            if let Some(ref summary) = entry.memory_summary {
                let truncated = if summary.chars().count() > 200 {
                    let head: String = summary.chars().take(200).collect();
                    format!("{head}…")
                } else {
                    summary.clone()
                };
                out.push_str(&format!("\n  Recent context: {truncated}"));
            }
            out.push('\n');
        }
        Ok(out)
    }
}

impl PromptSection for IdentitySection {
    fn name(&self) -> &str {
        "identity"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        let mut prompt = String::from("## Project Context\n\n");
        prompt.push_str(
            "The following workspace files define your identity, behavior, and context.\n\n",
        );
        // When the visible-tool filter is active the main agent is a pure
        // orchestrator: it routes via spawn_subagent, synthesises results,
        // and talks to the user. It does NOT need the periodic-task config
        // (HEARTBEAT.md) — subagents handle their own concerns.
        let is_orchestrator = !ctx.visible_tool_names.is_empty();
        let all_files: &[&str] = &["SOUL.md", "IDENTITY.md", "HEARTBEAT.md"];
        // Orchestrator skips these from the prompt but we still sync them
        // to disk so they stay current.
        let skip_in_prompt: &[&str] = if is_orchestrator {
            &["HEARTBEAT.md"]
        } else {
            &[]
        };
        for file in all_files {
            // Always sync to disk so builtin updates ship.
            sync_workspace_file(ctx.workspace_dir, file);
            if skip_in_prompt.contains(file) {
                continue;
            }
            if *file == "SOUL.md" {
                if let Some(ref soul) = ctx.personality_soul_md {
                    tracing::debug!(
                        "[identity] personality SOUL.md override active ({} chars)",
                        soul.len()
                    );
                    inject_inline_content(&mut prompt, "SOUL.md", soul, BOOTSTRAP_MAX_CHARS);
                    continue;
                }
            }
            inject_workspace_file(&mut prompt, ctx.workspace_dir, file);
        }

        // Seed MEMORY_GOALS.md to disk (header-only default) so the
        // long-term goals list is discoverable in the workspace from first
        // boot. Sync-only: the goals file is deliberately NOT injected into
        // the system prompt — it is stored state managed by the memory_goals
        // domain (RPC / tools / enrichment agent).
        sync_workspace_file(ctx.workspace_dir, "MEMORY_GOALS.md");

        // PROFILE.md / MEMORY.md injection lives in the dedicated
        // `UserFilesSection` (below) so agents that strip the identity
        // preamble (`omit_identity = true`) — welcome, orchestrator, the
        // trigger pair — still get their user files at runtime via
        // `SystemPromptBuilder::for_subagent`, which omits
        // `IdentitySection` entirely when `omit_identity` is set.

        Ok(prompt)
    }
}

impl PromptSection for UserFilesSection {
    fn name(&self) -> &str {
        "user_files"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        // Gate on the per-agent flags derived from
        // `AgentDefinition::omit_profile` / `omit_memory_md`. Both files
        // are user-specific, potentially growing, and capped at
        // [`USER_FILE_MAX_CHARS`] (~1000 tokens) so they can't bloat the
        // cached prefix.
        //
        // KV-cache contract: once injected into a session's rendered
        // prompt, the bytes are frozen for the remainder of that
        // session — any mid-session archivist write or enrichment
        // refresh lands on the NEXT session, never the in-flight one.
        let mut out = String::new();
        if ctx.include_profile {
            inject_workspace_file_capped(
                &mut out,
                ctx.workspace_dir,
                "PROFILE.md",
                USER_FILE_MAX_CHARS,
            );
        }
        if ctx.include_memory_md {
            // Personality-specific MEMORY.md takes highest priority, then
            // the session-frozen curated-memory snapshot, then the
            // workspace file (pure prompt-unit tests and older call sites).
            if let Some(ref memory_md) = ctx.personality_memory_md {
                tracing::debug!(
                    "[user_files] personality MEMORY.md override active ({} chars)",
                    memory_md.len()
                );
                inject_inline_content(&mut out, "MEMORY.md", memory_md, USER_FILE_MAX_CHARS);
            } else if let Some(snap) = &ctx.curated_snapshot {
                inject_snapshot_content(&mut out, "MEMORY.md", &snap.memory, USER_FILE_MAX_CHARS);
                inject_snapshot_content(&mut out, "USER.md", &snap.user, USER_FILE_MAX_CHARS);
            } else {
                inject_workspace_file_capped(
                    &mut out,
                    ctx.workspace_dir,
                    "MEMORY.md",
                    USER_FILE_MAX_CHARS,
                );
            }
        }
        Ok(out)
    }
}

impl PromptSection for ToolsSection {
    fn name(&self) -> &str {
        "tools"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        // Native function-calling: the provider already sends full JSON
        // schemas in the API request — no need to repeat the tool catalogue
        // in the system prompt (pure token bloat). However, any non-empty
        // `dispatcher_instructions` (e.g. the "## Tool Use Protocol" block
        // from NativeToolDispatcher) must still be included so the model
        // receives its behavioural guidance.
        if ctx.tool_call_format == ToolCallFormat::Native {
            if ctx.dispatcher_instructions.trim().is_empty() {
                return Ok(String::new());
            }
            return Ok(ctx.dispatcher_instructions.to_string());
        }
        let mut out = String::from("## Tools\n\n");
        let has_filter = !ctx.visible_tool_names.is_empty();
        for tool in ctx.tools {
            // Skip tools not in the visible set when a filter is active.
            if has_filter && !ctx.visible_tool_names.contains(tool.name) {
                continue;
            }

            // One rendering shape for every dispatcher: a compact
            // P-Format signature (`name[a|b|c]`). The signature comes
            // straight from the parameter schema (alphabetical by
            // property name — see `pformat` module docs for why) so
            // model and parser agree on argument ordering. For
            // `Native` dispatchers the provider already has the full
            // JSON schema in the API request, so repeating it in the
            // prompt is pure token bloat; for `Json` / `PFormat` text
            // dispatchers the dispatcher's own `prompt_instructions`
            // block (appended below) carries whatever schema detail
            // the wire format needs.
            let signature = render_pformat_signature_for_prompt(tool);
            let _ = writeln!(
                out,
                "- **{}**: {}\n  Call as: `{}`",
                tool.name, tool.description, signature
            );
        }
        if !ctx.dispatcher_instructions.is_empty() {
            out.push('\n');
            out.push_str(ctx.dispatcher_instructions);
        }
        Ok(out)
    }
}

impl PromptSection for SafetySection {
    fn name(&self) -> &str {
        "safety"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        Ok("## Safety\n\n- Do not exfiltrate private data.\n- Do not run destructive commands without asking.\n- Do not bypass oversight or approval mechanisms.\n- Prefer `trash` over `rm`.\n- When in doubt, ask before acting externally.".into())
    }
}

/// Canonical grounding / anti-hallucination contract.
///
/// This is the **single source of truth** for the tool-use and
/// anti-fabrication rules every agent inherits. Before this block existed,
/// the same "never invent ids / a tool not in your list does not exist"
/// paragraph was copy-pasted (and slowly drifting) across crypto, markets,
/// integrations, account-admin, mcp-setup, morning-briefing, researcher, …
/// agent prompts. Centralising it kills that drift and guarantees a uniform
/// floor of grounding discipline.
///
/// Inspired by Hermes's named guidance blocks (`TOOL_USE_ENFORCEMENT_GUIDANCE`
/// "use tools to act, never end a turn with a promise", `TASK_COMPLETION_GUIDANCE`
/// "never substitute fabricated output for results you couldn't produce").
///
/// Deliberately **generic**: every clause must be true for *every* agent,
/// including the integrations executor. Agent-specific routing (e.g. "delegate
/// external services", "pull slugs from `composio_list_tools`") stays in that
/// agent's own `prompt.md`, not here.
///
/// Byte-stable (no time / RNG / host) so it lives in the KV-cache-friendly
/// prefix. Must contain no em-dashes per [`super::builder::GLOBAL_STYLE_SUFFIX`].
pub const GROUNDING_BODY: &str = "## Grounding and tool use\n\n\
    - Your tools are exactly the ones listed in this prompt. You can only act through them. If a capability is not one of your tools, say so plainly rather than pretending it exists.\n\
    - Never invent tool names, arguments, ids, slugs, file paths, URLs, chain ids, addresses, quotes, metrics, or any other value. If you do not have it from a tool result or the user, ask for it or look it up with a tool.\n\
    - Preserve numeric evidence exactly. For numbers, counts, sizes, dates, timestamps, durations, currencies, percentages, quotas, and ids, copy the exact value from the observed tool result, user message, or cited memory into your answer.\n\
    - Do not round, convert units, rewrite relative times, or recalculate numeric values unless the user asks and you show the calculation from observed values. If sources disagree, name the discrepancy instead of choosing a plausible value.\n\
    - Use your tools to act. Do not just describe what you would do and stop, and never end a turn with a promise of future action: do it now, or hand back a concrete result.\n\
    - Never substitute plausible looking but fabricated output (made up data, invented file contents, synthesised tool or API responses) for results you could not actually produce. If a step failed, say it failed.\n\
    - Ground every factual claim in evidence you actually observed: a tool result, the user's message, or cited memory. If the evidence is missing, partial, or truncated, say so or fetch more instead of guessing.\n\
    - Skills run only via `run_workflow`, and only the skills listed as installed exist. Do not invent skill ids.";

impl PromptSection for GroundingSection {
    fn name(&self) -> &str {
        "grounding"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        Ok(GROUNDING_BODY.into())
    }
}

impl PromptSection for WorkspaceSection {
    fn name(&self) -> &str {
        "workspace"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        // Intentionally does NOT print a hardcoded path: `shell` and the file
        // tools resolve relative paths against the agent's *action directory*,
        // which is not `workspace_dir`. Printing `workspace_dir` here used to
        // point agents at a directory the file tools are sandboxed *out of*,
        // causing write→read mismatches. Instead, tell the agent to discover
        // its real working directory at runtime and keep writes/reads there.
        let mut out = String::from(
            "## Workspace\n\n\
             Run `pwd` to confirm your working directory — that is where `shell` runs and \
             where `file_read`/`file_write` resolve relative paths. Create files in that \
             directory and read them back from the same place (use the relative path, or \
             confirm the absolute path with `pwd`). Writes and reads outside your granted \
             locations (your working directory plus the scratch directory below) are blocked \
             by the security sandbox.\n\n\
             Prefer printing results to stdout. Only when output is too large for stdout, \
             write it to a file in your working directory and read that file back.\n\n",
        );
        // Only advertise a concrete scratch path when the dir is actually present
        // and safe (real dir, not a symlink) — matching the policy grant in
        // `SecurityPolicy::from_config`. Otherwise fall back to env-var/working-dir
        // wording so we never point file I/O at a location the sandbox would block.
        // Read-only check (no fs side effects in prompt rendering).
        let scratch = crate::openhuman::security::openhuman_scratch_dir();
        let scratch_granted = std::fs::symlink_metadata(&scratch)
            .map(|m| !m.file_type().is_symlink() && m.is_dir())
            .unwrap_or(false);
        if scratch_granted {
            let _ = write!(
                out,
                "For scratch or temporary files, use the directory `{}` (a granted scratch \
                 space) or your `$TMPDIR` / `%TEMP%` — never a hardcoded `/tmp/<name>` path.",
                scratch.display()
            );
        } else {
            out.push_str(
                "For scratch or temporary files, use `$TMPDIR` / `%TEMP%`, or create them in \
                 your working directory — never a hardcoded `/tmp/<name>` path, which is blocked.",
            );
        }
        Ok(out)
    }
}

impl PromptSection for RuntimeSection {
    fn name(&self) -> &str {
        "runtime"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        let host =
            hostname::get().map_or_else(|_| "unknown".into(), |h| h.to_string_lossy().to_string());
        Ok(format!(
            "## Runtime\n\nHost: {host} | OS: {} | Model: {}",
            std::env::consts::OS,
            ctx.model_name
        ))
    }
}

impl PromptSection for UserReflectionsSection {
    fn name(&self) -> &str {
        "user_reflections"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        if ctx.learned.reflections.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("## User Reflections\n\n");
        out.push_str(
            "Explicit reflections the user authored about themselves, their goals, \
             or how they want you to behave going forward. Treat these as \
             higher-priority than the broader user-memory summaries below: \
             they are recent, intentional, identity-relevant signals and \
             should steer your responses ahead of any generic historical \
             context.\n\n",
        );
        for reflection in &ctx.learned.reflections {
            let trimmed = reflection.trim();
            if trimmed.is_empty() {
                continue;
            }
            out.push_str("- ");
            out.push_str(trimmed);
            out.push('\n');
        }
        out.push('\n');
        Ok(out)
    }
}

impl PromptSection for UserMemorySection {
    fn name(&self) -> &str {
        "user_memory"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        if ctx.learned.tree_root_summaries.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("## User Memory\n\n");
        out.push_str(
            "Long-term memory distilled by the tree summarizer. \
             Each section is the root summary for a memory namespace, \
             representing everything we've learned about that domain over time. \
             Treat this as durable background context, but NOT as fresh, \
             present-tense fact: each section header shows when that memory \
             was last updated. Compare those dates against the `## Current \
             Date & Time` section below before answering time-sensitive \
             questions (today's briefing, daily summary, reminders, calendar, \
             notifications, \"today/tomorrow/this week\"). If a summary predates \
             the period the user is asking about, treat it as potentially \
             stale — say so explicitly and never present older memory as \
             today's update.\n\n",
        );

        for NamespaceSummary {
            namespace,
            body,
            updated_at,
        } in &ctx.learned.tree_root_summaries
        {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Absolute date (not "N days ago") keeps this front-of-prompt
            // section byte-stable for KV-cache reuse — see `NamespaceSummary`.
            let _ = writeln!(
                out,
                "### {namespace} (last updated {})\n",
                super::render_helpers::memory_date_label(*updated_at)
            );
            out.push_str(trimmed);
            out.push_str("\n\n");
        }

        Ok(out)
    }
}

impl PromptSection for DateTimeSection {
    fn name(&self) -> &str {
        "datetime"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        // No concrete timestamp here. The live "now" is injected per turn
        // on the user message via `render_helpers::current_datetime_line`
        // (the main session loop and the sub-agent runner both do this), so
        // it stays fresh across a long-lived session instead of freezing
        // into the cached system-prompt prefix and going stale (#3602).
        // This section carries only the *static* discipline rules for using
        // that stamp — keeping the prefix byte-stable for KV-cache reuse.
        //
        // Greeting/clock grounding is ungated: every agent that emits a
        // time-relative word (a greeting, "today"/"tonight") needs it,
        // whether or not it owns `resolve_time`. Without this rule the model
        // treats the time line as passive reference and defaults to a
        // learned "good morning" regardless of the actual hour (#3602).
        let mut out = String::from(
            "## Current Date & Time\n\n> The current local date and time is provided on a \
             `Current Date & Time:` line with the latest message (local time, IANA timezone, \
             UTC offset, weekday). Before any time-relative wording in your reply — greetings \
             like \"good morning\"/\"good evening\", or \"today\"/\"tonight\"/\"tomorrow\" — read \
             that line and match the actual local hour. Never assume it is morning. The time is \
             already in context; no tool call is needed to greet or to reason about the current day.",
        );
        // Tool-argument discipline, gated on the agent actually having the
        // `resolve_time` tool. LLMs are unreliable at epoch arithmetic — a
        // real incident had an agent compute "24h ago" ~10 months off, then
        // fetch Slack history ascending from that wrong floor and never reach
        // the latest messages. Telling agents to lean on the tool (rather
        // than hand-computing) is the fix. Auto-scopes: agents without the
        // tool never see the rule.
        if ctx.tools.iter().any(|t| t.name == "resolve_time") {
            out.push_str(
                "\n\n> For any date/time you pass as a tool argument \
                 (`oldest`/`latest`/`since`/`after`, cron times, etc.), call \
                 `resolve_time` and use its exact value — never hand-compute \
                 epoch/Unix seconds. For \"recent / last N\" lookups, prefer \
                 newest-first (omit `oldest`) so a wrong floor can't bury the \
                 latest data.",
            );
        }
        Ok(out)
    }
}

impl PromptSection for UserIdentitySection {
    fn name(&self) -> &str {
        "user_identity"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        let identity = match ctx.user_identity.as_ref() {
            Some(id) if !id.is_empty() => id,
            _ => return Ok(String::new()),
        };

        // Render the field list FIRST, then decide whether to ship the
        // heading. `UserIdentity::is_empty()` only checks `None`-ness —
        // a struct whose fields are all `Some("")` / whitespace would
        // otherwise leave the prompt with a `## User` heading + intro
        // pointing at zero fields, which is exactly the empty-prompt
        // failure mode we're trying to suppress (#926).
        let mut fields = String::new();
        if let Some(name) = identity.name.as_deref().filter(|s| !s.trim().is_empty()) {
            let _ = writeln!(fields, "- name: {}", sanitize_identity_field(name));
        }
        if let Some(email) = identity.email.as_deref().filter(|s| !s.trim().is_empty()) {
            let _ = writeln!(fields, "- email: {}", sanitize_identity_field(email));
        }
        if let Some(id) = identity.id.as_deref().filter(|s| !s.trim().is_empty()) {
            let _ = writeln!(fields, "- id: {}", sanitize_identity_field(id));
        }
        if fields.trim().is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("## User\n\n");
        out.push_str(
            "The signed-in user is identified below. Use these fields directly in tool \
             calls and do not ask the user to repeat them.\n\n",
        );
        out.push_str(&fields);
        Ok(out.trim_end().to_string())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Private helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Collapse newlines and runs of whitespace in a user-identity field so
/// it fits on a single markdown bullet without breaking the prompt
/// structure. Values come from `auth_get_me` (server-controlled), but
/// defence-in-depth: a name with embedded newlines could split the
/// `- name:` bullet and reshape the `## User` block.
fn sanitize_identity_field(s: &str) -> String {
    s.chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build a P-Format signature line (`name[a|b|c]`) from a [`PromptTool`].
/// Local to this module so [`ToolsSection`] doesn't have to depend on
/// the agent crate's `pformat` helper. The two implementations stay in
/// lockstep — both use BTreeMap iteration order on the schema's
/// `properties` field.
fn render_pformat_signature_for_prompt(tool: &PromptTool<'_>) -> String {
    let names: Vec<String> = tool
        .parameters_schema
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| {
            v.get("properties")
                .and_then(|p| p.as_object())
                .map(|m| m.keys().cloned().collect())
        })
        .unwrap_or_default();
    if names.is_empty() {
        format!("{}[]", tool.name)
    } else {
        format!("{}[{}]", tool.name, names.join("|"))
    }
}
