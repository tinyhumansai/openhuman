//! System prompt builder for the `morning_briefing` built-in agent.
//!
//! Returns the fully-assembled system prompt. Each agent's `build()`
//! composes section helpers from [`crate::openhuman::context::prompt`]
//! in the order it wants — so the output IS what the LLM sees, no
//! post-processing in the runner.

use crate::openhuman::context::prompt::{
    render_ambient_environment, render_tools, render_user_files, render_workspace, PromptContext,
};
use anyhow::Result;

const ARCHETYPE: &str = include_str!("prompt.md");

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    let mut out = String::with_capacity(4096);
    out.push_str(ARCHETYPE.trim_end());
    out.push_str("\n\n");

    let user_files = render_user_files(ctx)?;
    if !user_files.trim().is_empty() {
        out.push_str(user_files.trim_end());
        out.push_str("\n\n");
    }

    let tools = render_tools(ctx)?;
    if !tools.trim().is_empty() {
        out.push_str(tools.trim_end());
        out.push_str("\n\n");
    }

    let workspace = render_workspace(ctx)?;
    if !workspace.trim().is_empty() {
        out.push_str(workspace.trim_end());
        out.push_str("\n\n");
    }

    // Ambient runtime + user identity + current date/time so the
    // briefing agent stops asking the user "what timezone are you in?"
    // when the desktop app already knows — issue #926. Block sits at
    // the prompt tail because the embedded `Local::now()` makes it
    // time-volatile, matching the KV cache convention from
    // `SystemPromptBuilder::with_defaults`.
    let ambient = render_ambient_environment(ctx)?;
    if !ambient.trim().is_empty() {
        out.push_str(ambient.trim_end());
        out.push('\n');
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::context::prompt::{LearnedContextData, ToolCallFormat, UserIdentity};
    use std::collections::HashSet;

    fn ctx_with_identity(identity: Option<UserIdentity>) -> PromptContext<'static> {
        // SAFETY note: the empty visible-set is leaked once via a
        // `Box::leak` so it can satisfy the `'static` lifetime on the
        // returned context — these tests are short-lived and the
        // singleton allocation costs nothing on the hot path.
        let visible: &'static HashSet<String> = Box::leak(Box::new(HashSet::new()));
        PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "morning_briefing",
            tools: &[],
            workflows: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: visible,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            connected_identities_md: String::new(),
            include_profile: false,
            include_memory_md: false,
            curated_snapshot: None,
            user_identity: identity,
            personality_soul_md: None,
            personality_memory_md: None,
            personality_roster: vec![],
        }
    }

    #[test]
    fn build_returns_nonempty_body() {
        let body = build(&ctx_with_identity(None)).unwrap();
        assert!(!body.is_empty());
    }

    #[test]
    fn build_includes_runtime_and_datetime_sections() {
        // Issue #926 + #3602: the morning briefing must carry the `## Runtime`
        // host block and the `## Current Date & Time` grounding section so the
        // agent never asks the user "what timezone are you in?" and grounds its
        // greeting on the real clock. The concrete "now" itself rides the
        // per-turn user message (`current_datetime_line`) — see #3602 — so this
        // pins the *section wiring*, not a volatile timestamp.
        let body = build(&ctx_with_identity(None)).unwrap();
        assert!(
            body.contains("## Runtime"),
            "morning_briefing prompt must carry `## Runtime` (host + OS) so the model \
             knows which device it's on; got:\n{body}"
        );
        assert!(
            body.contains("## Current Date & Time"),
            "morning_briefing prompt must carry `## Current Date & Time` (#926); got:\n{body}"
        );
        // The grounding rule must reach the model so it matches greetings to
        // the actual local hour (#3602). The live clock is injected per turn,
        // so we assert the rule, not a baked-in timestamp.
        let dt = body
            .split("## Current Date & Time")
            .nth(1)
            .expect("datetime section must follow its heading");
        assert!(
            dt.contains("match the actual local hour"),
            "datetime section must carry the greeting-grounding rule (#3602); got:\n{dt}"
        );
    }

    #[test]
    fn build_includes_user_identity_when_present() {
        // When the auth cache has populated `user_identity`, the
        // briefing prompt must surface those fields so the agent can
        // greet the user by name and address mail without asking.
        let identity = UserIdentity {
            id: Some("u_42".to_string()),
            name: Some("Ada Lovelace".to_string()),
            email: Some("ada@example.com".to_string()),
        };
        let body = build(&ctx_with_identity(Some(identity))).unwrap();
        assert!(body.contains("## User"));
        assert!(body.contains("- name: Ada Lovelace"));
        assert!(body.contains("- email: ada@example.com"));
        // The `## User` block must NEVER carry token / refresh fields —
        // only id / name / email by construction. Sanity-check here so
        // a future field addition forces a deliberate test update.
        assert!(
            !body.to_lowercase().contains("token"),
            "user identity block must never embed token fields; got:\n{body}"
        );
    }

    #[test]
    fn build_omits_user_section_when_identity_unset() {
        let body = build(&ctx_with_identity(None)).unwrap();
        assert!(
            !body.contains("## User\n"),
            "user section must be empty when no auth cache is populated (CLI flows, \
             signed-out sessions); got:\n{body}"
        );
    }
}
