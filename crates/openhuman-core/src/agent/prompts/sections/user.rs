//! The user-scoped sections: learned memory, explicit reflections, and the
//! signed-in identity. Split out of `sections.rs` to keep it under the
//! layout gate's line limit; re-exported from there unchanged.

use super::super::types::*;
use anyhow::Result;
use std::fmt::Write;

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

impl PromptSection for UserReflectionsSection {
    fn tier(&self) -> PromptTier {
        // Learned reflections, refreshed by the learning subsystem.
        PromptTier::Volatile
    }

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
    fn tier(&self) -> PromptTier {
        // The memory-tree summary, which moves on every memory write.
        PromptTier::Volatile
    }

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
                super::super::render_helpers::memory_date_label(*updated_at)
            );
            out.push_str(trimmed);
            out.push_str("\n\n");
        }

        Ok(out)
    }
}

impl PromptSection for UserIdentitySection {
    fn tier(&self) -> PromptTier {
        // The signed-in user, which changes on login and on logout.
        PromptTier::Volatile
    }

    fn name(&self) -> &str {
        "user_identity"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        let identity = match ctx.user_identity.as_ref() {
            Some(id) if !id.is_empty() => id,
            _ => return Ok(String::new()),
        };

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

/// Collapse whitespace in a user-identity field for a single markdown bullet.
fn sanitize_identity_field(s: &str) -> String {
    s.chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
