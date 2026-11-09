//! Functional `render_*` wrappers over the section structs in
//! [`crate::prompts::sections`], plus the per-turn datetime stamp and the
//! ambient-environment composer that `agents/<id>/prompt.rs` builders call.

use crate::agent::prompts::types::*;
use crate::prompts::sections::*;
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::sync::{Arc, OnceLock};
use tinytools_agent::dialect::ToolCallFormat;

// ─────────────────────────────────────────────────────────────────────────────
// Section render helpers (functional wrappers over section structs)
// ─────────────────────────────────────────────────────────────────────────────

/// Render the `## Project Context` identity block
/// (`SOUL.md` / `IDENTITY.md` / `ROLE.md` for the user-facing agent).
pub fn render_identity(ctx: &PromptContext<'_>) -> Result<String> {
    IdentitySection.build(ctx)
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

// `render_skills` and `render_connected_integrations` helpers are
// gone — the connected-integrations / delegation-guide blocks each live in
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

type PromptClock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

static PROMPT_CLOCKS: crate::core::slots::HookSlot<PromptClock> =
    crate::core::slots::HookSlot::new("openhuman.prompts.clock");
static NEXT_PROMPT_CLOCK_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A temporary prompt-clock registration, removed when this guard is dropped.
///
/// Registrations are process-scoped and selected newest-first. Keeping this
/// guard on the registering thread also keeps the test build's thread-local
/// HookSlot registration and its removal paired.
pub struct PromptClockRegistration {
    id: String,
    _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Drop for PromptClockRegistration {
    fn drop(&mut self) {
        PROMPT_CLOCKS.unregister(&self.id);
    }
}

/// Temporarily provide the instant used by per-turn prompt date lines.
///
/// This is process-scoped; the most recently registered live clock wins, and
/// dropping registrations in any order restores the newest remaining clock.
/// With no registration, prompt rendering uses the system clock.
pub fn register_prompt_clock(
    clock: impl Fn() -> DateTime<Utc> + Send + Sync + 'static,
) -> PromptClockRegistration {
    let id = format!(
        "{}",
        NEXT_PROMPT_CLOCK_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    PROMPT_CLOCKS.register(id.clone(), Arc::new(clock));
    PromptClockRegistration {
        id,
        _not_send: std::marker::PhantomData,
    }
}

/// Render the canonical one-line timestamp at a supplied UTC instant.
///
/// The zone and profile-force-UTC behavior match [`current_datetime_line`].
pub fn current_datetime_line_at(now: DateTime<Utc>, zone: Option<&str>) -> String {
    // `library-cpu.sh` sets `OPENHUMAN_PROFILE_FORCE_UTC=1` to skip
    // `iana_time_zone`/CoreFoundation timezone resolution, which is itself a
    // measurable cost in a cold CPU profile. Gated on `rss-bench`, so it does
    // not exist in any shipped build.
    #[cfg(feature = "rss-bench")]
    if std::env::var_os("OPENHUMAN_PROFILE_FORCE_UTC").is_some() {
        return format!(
            "Current Date & Time: {} UTC (UTC, UTC+00:00), {}",
            now.format("%Y-%m-%d %H:%M:%S"),
            now.format("%A"),
        );
    }

    if let Some(tz) = zone.and_then(|zone| zone.parse::<chrono_tz::Tz>().ok()) {
        let now = now.with_timezone(&tz);
        return format!(
            "Current Date & Time: {} {} ({}, UTC{}), {}",
            now.format("%Y-%m-%d %H:%M:%S"),
            tz.name(),
            now.format("%Z"),
            now.format("%:z"),
            now.format("%A"),
        );
    }

    // When the host resolves an IANA zone, stamp local time + that zone. When
    // it can't (CI, stripped containers), fall back to true UTC so the time,
    // offset, and zone label all agree rather than pairing a "UTC" label with
    // a local clock/offset.
    match iana_time_zone::get_timezone() {
        Ok(iana) => {
            let now = now.with_timezone(&chrono::Local);
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
            format!(
                "Current Date & Time: {} UTC (UTC, UTC+00:00), {}",
                now.format("%Y-%m-%d %H:%M:%S"),
                now.format("%A"),
            )
        }
    }
}

/// Canonical one-line "now" stamp, injected per turn alongside the user
/// message by both the main session loop (`session::turn`) and the
/// sub-agent runner so every flow reports the current time identically
/// (#3602). Local time + IANA zone + `%Z`/offset + weekday, so the model
/// can localize greetings and date math without a tool call.
///
/// Deliberately lives on the *user message*, never the cached
/// system-prompt prefix: the clock is volatile, so freezing it into the
/// prefix both busts the KV cache and goes stale across a long-lived
/// session. The static grounding *rule* that tells the model to read this
/// line lives in [`DateTimeSection`] / [`render_datetime`].
///
/// `zone` is the user's IANA zone ([`crate::config::Config::time_zone`]);
/// `None`, or a name chrono-tz does not know, stamps the device's. The
/// newest live prompt-clock registration is used when present; otherwise
/// this reads the system clock.
pub fn current_datetime_line(zone: Option<&str>) -> String {
    let now = PROMPT_CLOCKS
        .entries()
        .last()
        .map(|(_, clock)| clock())
        .unwrap_or_else(Utc::now);
    current_datetime_line_at(now, zone)
}

/// What leads every user message: [`current_datetime_line`], then the
/// reply-language instruction for the user's interface locale, when there is
/// one. Per message, never in the cached prompt prefix, so a locale change
/// applies from the next turn and the prefix stays byte-stable.
pub fn turn_preamble(reply_language_directive: Option<&str>, zone: Option<&str>) -> String {
    let now = current_datetime_line(zone);
    match reply_language_directive {
        Some(directive) => format!("{now}\n{directive}"),
        None => now,
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
/// by [`crate::prompts::SystemPromptBuilder::with_defaults`].
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

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Build a throwaway `PromptContext` for sections whose `build` only
/// uses static/immutable inputs (currently just `SafetySection`). Keeps
/// the `render_safety()` free function from forcing callers to
/// manufacture a full context when they only need the static text.
fn empty_prompt_context_for_static_sections() -> PromptContext<'static> {
    static EMPTY_TOOLS: &[PromptTool<'static>] = &[];
    static EMPTY_WORKFLOWS: &[tinyskills::Skill] = &[];
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
        visible_tool_names: visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: EMPTY_INTEGRATIONS,
        connected_identities_md: String::new(),
        user_identity: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
        flow_archetype_text: None,
    }
}

#[cfg(test)]
#[path = "section_renderers_tests.rs"]
mod tests;
