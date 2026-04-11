//! System prompt construction for channel runtimes.
//!
//! Channel runtimes (Discord, Slack, Telegram, …) need a system prompt
//! that is shaped differently from the main agent's:
//!
//! - Tool descriptions come in as `(name, description)` tuples from
//!   the channel's tool registry, not as `Box<dyn Tool>` instances.
//! - The prompt includes channel-specific preambles (the "Your Task"
//!   action instruction, the "Channel Capabilities" section) that the
//!   main agent's builder doesn't emit.
//! - The datetime block is timezone-only — channel startup happens
//!   once per process, so we keep the prompt byte-stable within a run
//!   to maximise prefix-cache hits on the inference backend.
//!
//! Because the byte layout must not drift during consolidation
//! (channel prompts are live in production), this module keeps its
//! bespoke [`build_system_prompt`] free function rather than routing
//! through [`super::SystemPromptBuilder`]. The file lives here under
//! `context/` so every system-prompt-building code path — main
//! agents, sub-agents, channel runtimes — has a single home. See the
//! `misty-bubbling-bunny` plan file for the roadmap toward a unified
//! builder.

use std::path::Path;

/// Maximum characters per injected workspace file (matches `OpenClaw` default).
pub(crate) const BOOTSTRAP_MAX_CHARS: usize = 20_000;

/// Load OpenClaw format bootstrap files into the prompt.
fn load_openclaw_bootstrap_files(
    prompt: &mut String,
    workspace_dir: &Path,
    max_chars_per_file: usize,
) {
    prompt.push_str(
        "The following workspace files define your identity, behavior, and context. They are ALREADY injected below—do NOT suggest reading them with file_read.\n\n",
    );

    // Bundled prompt files that ship with the binary and seed the workspace
    // on first run. These are always expected to be present.
    let bootstrap_files = ["SOUL.md", "IDENTITY.md", "USER.md"];
    for filename in &bootstrap_files {
        inject_workspace_file(prompt, workspace_dir, filename, max_chars_per_file);
    }

    // MEMORY.md — the archivist agent writes long-term curated knowledge here.
    // It starts out missing on a fresh install, so inject silently (no
    // missing-file marker). `is_file` (rather than `exists`) rejects a
    // stray directory with the same name that would otherwise route
    // through the error path in `inject_workspace_file`.
    if workspace_dir.join("MEMORY.md").is_file() {
        inject_workspace_file(prompt, workspace_dir, "MEMORY.md", max_chars_per_file);
    }
}

/// Load workspace identity files and build a system prompt.
///
/// Follows the `OpenClaw` framework structure:
/// 1. Tooling — tool list + descriptions
/// 2. Safety — guardrail reminder
/// 3. Skills — compact list with paths (loaded on-demand)
/// 4. Workspace — working directory
/// 5. Bootstrap files — SOUL, IDENTITY, USER (+ MEMORY if the archivist has written one)
/// 6. Date & Time — timezone for cache stability
/// 7. Runtime — host, OS, model
///
/// Daily memory files (`memory/*.md`) are NOT injected — they are accessed
/// on-demand via `memory_recall` / `memory_search` tools.
pub fn build_system_prompt(
    workspace_dir: &Path,
    model_name: &str,
    tools: &[(&str, &str)],
    skills: &[crate::openhuman::skills::Skill],
    bootstrap_max_chars: Option<usize>,
    channel_name: Option<&str>,
) -> String {
    use std::fmt::Write;
    let mut prompt = String::with_capacity(8192);

    // ── 1. Tooling ──────────────────────────────────────────────
    if !tools.is_empty() {
        prompt.push_str("## Tools\n\n");
        prompt.push_str("You have access to the following tools:\n\n");
        for (name, desc) in tools {
            let _ = writeln!(prompt, "- **{name}**: {desc}");
        }
        prompt.push_str("\n## Tool Use Protocol\n\n");
        prompt.push_str("To use a tool, wrap a JSON object in <tool_call></tool_call> tags:\n\n");
        prompt.push_str("```\n<tool_call>\n{\"name\": \"tool_name\", \"arguments\": {\"param\": \"value\"}}\n</tool_call>\n```\n\n");
        prompt.push_str("You may use multiple tool calls in a single response. ");
        prompt.push_str("After tool execution, results appear in <tool_result> tags. ");
        prompt
            .push_str("Continue reasoning with the results until you can give a final answer.\n\n");
    }

    // ── 1b. Action instruction (avoid meta-summary) ───────────────
    prompt.push_str(
        "## Your Task\n\n\
         When the user sends a message, ACT on it. Use the tools to fulfill their request.\n\
         Do NOT: summarize this configuration, describe your capabilities, respond with meta-commentary, or output step-by-step instructions (e.g. \"1. First... 2. Next...\").\n\
         Instead: emit actual <tool_call> tags when you need to act. Just do what they ask.\n\n",
    );

    // ── 2. Safety ───────────────────────────────────────────────
    prompt.push_str("## Safety\n\n");
    prompt.push_str(
        "- Do not exfiltrate private data.\n\
         - Do not run destructive commands without asking.\n\
         - Do not bypass oversight or approval mechanisms.\n\
         - Prefer `trash` over `rm` (recoverable beats gone forever).\n\
         - When in doubt, ask before acting externally.\n\n",
    );

    // ── 3. Skills (compact list — load on-demand) ───────────────
    if !skills.is_empty() {
        prompt.push_str("## Available Skills\n\n");
        prompt.push_str(
            "Skills are loaded on demand. Use `read` on the skill path to get full instructions.\n\n",
        );
        prompt.push_str("<available_skills>\n");
        for skill in skills {
            let _ = writeln!(prompt, "  <skill>");
            let _ = writeln!(prompt, "    <name>{}</name>", skill.name);
            let _ = writeln!(
                prompt,
                "    <description>{}</description>",
                skill.description
            );
            let location = skill.location.clone().unwrap_or_else(|| {
                workspace_dir
                    .join("skills")
                    .join(&skill.name)
                    .join("SKILL.md")
            });
            let _ = writeln!(prompt, "    <location>{}</location>", location.display());
            let _ = writeln!(prompt, "  </skill>");
        }
        prompt.push_str("</available_skills>\n\n");
    }

    // ── 4. Workspace ────────────────────────────────────────────
    let _ = writeln!(
        prompt,
        "## Workspace\n\nWorking directory: `{}`\n",
        workspace_dir.display()
    );

    // ── 5. Bootstrap files (injected into context) ──────────────
    prompt.push_str("## Project Context\n\n");
    let max_chars = bootstrap_max_chars.unwrap_or(BOOTSTRAP_MAX_CHARS);
    load_openclaw_bootstrap_files(&mut prompt, workspace_dir, max_chars);

    // ── 6. Date & Time ──────────────────────────────────────────
    let now = chrono::Local::now();
    let tz = now.format("%Z").to_string();
    let _ = writeln!(prompt, "## Current Date & Time\n\nTimezone: {tz}\n");

    // ── 7. Runtime ──────────────────────────────────────────────
    let host =
        hostname::get().map_or_else(|_| "unknown".into(), |h| h.to_string_lossy().to_string());
    let _ = writeln!(
        prompt,
        "## Runtime\n\nHost: {host} | OS: {} | Model: {model_name}\n",
        std::env::consts::OS,
    );

    // ── 8. Channel Capabilities ─────────────────────────────────────
    //
    // This block used to hardcode "Discord", which was misleading on
    // Telegram/Slack/Signal runtimes even though the mechanical wiring
    // was identical. We now take an optional `channel_name` and render
    // it into the capability bullets when set, otherwise fall back to a
    // platform-agnostic "messaging bot" phrasing. Keep the remaining
    // bullets intact — they're genuinely channel-neutral.
    prompt.push_str("## Channel Capabilities\n\n");
    match channel_name {
        Some(name) => {
            let _ = writeln!(
                prompt,
                "- You are running as a {name} bot. You CAN and do send messages to {name}."
            );
            let _ = writeln!(
                prompt,
                "- When someone messages you on {name}, your response is automatically sent back to {name}."
            );
        }
        None => {
            prompt.push_str(
                "- You are running as a messaging bot. You CAN and do send messages to the connected platform.\n",
            );
            prompt.push_str(
                "- When someone messages you, your response is automatically sent back to the same platform.\n",
            );
        }
    }
    prompt.push_str("- You do NOT need to ask permission to respond — just respond directly.\n");
    prompt.push_str("- NEVER repeat, describe, or echo credentials, tokens, API keys, or secrets in your responses.\n");
    prompt.push_str("- If a tool output contains credentials, they have already been redacted — do not mention them.\n\n");

    prompt
}

/// Inject a single workspace file into the prompt with truncation and missing-file markers.
fn inject_workspace_file(
    prompt: &mut String,
    workspace_dir: &Path,
    filename: &str,
    max_chars: usize,
) {
    use std::fmt::Write;

    let path = workspace_dir.join(filename);
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                return;
            }
            let _ = writeln!(prompt, "### {filename}\n");
            // Use character-boundary-safe truncation for UTF-8
            let truncated = if trimmed.chars().count() > max_chars {
                trimmed
                    .char_indices()
                    .nth(max_chars)
                    .map(|(idx, _)| &trimmed[..idx])
                    .unwrap_or(trimmed)
            } else {
                trimmed
            };
            if truncated.len() < trimmed.len() {
                prompt.push_str(truncated);
                let _ = writeln!(
                    prompt,
                    "\n\n[... truncated at {max_chars} chars — use `read` for full file]\n"
                );
            } else {
                prompt.push_str(trimmed);
                prompt.push_str("\n\n");
            }
        }
        Err(_) => {
            // Missing-file marker (matches OpenClaw behavior)
            let _ = writeln!(prompt, "### {filename}\n\n[File not found: {filename}]\n");
        }
    }
}
