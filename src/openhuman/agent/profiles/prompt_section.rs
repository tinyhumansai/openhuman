//! Prompt section that injects a profile's free-form persona blurb, plus the
//! optional cross-profile workspace notice (1b) that tells the model where its
//! dedicated workspace is and that other profiles' directories are off-limits.

use std::path::Path;

use crate::openhuman::agent::context::prompt::{PromptContext, PromptSection};
use anyhow::Result;

/// One-sentence system-prompt notice, mirroring hermes's cross-profile
/// disclosure: names the profile's dedicated workspace and states that other
/// profiles' directories are off-limits. Rendered only when a dedicated
/// workspace is active (the enforcement backstop is the guard in
/// [`crate::openhuman::agent::profiles::guard`]).
pub fn cross_profile_workspace_notice(profile_id: &str, workspace_path: &Path) -> String {
    format!(
        "Your dedicated workspace for profile `{profile_id}` is `{}`. Work only there; the \
         directories of other profiles are off-limits.",
        workspace_path.display()
    )
}

/// Renders a profile's `system_prompt_suffix` (or any free-form persona body)
/// as a `## Agent profile` block in the system prompt, optionally followed by
/// the cross-profile workspace notice.
pub struct AgentProfilePromptSection {
    body: String,
    workspace_notice: Option<String>,
}

impl AgentProfilePromptSection {
    pub fn new(body: String) -> Self {
        Self {
            body,
            workspace_notice: None,
        }
    }

    /// Attach the cross-profile workspace notice (1b). Rendered under the
    /// persona body — or on its own when the body is empty — so a
    /// dedicated-workspace profile always discloses its boundary even without a
    /// custom persona suffix.
    #[must_use]
    pub fn with_workspace_notice(mut self, notice: String) -> Self {
        let notice = notice.trim().to_string();
        self.workspace_notice = (!notice.is_empty()).then_some(notice);
        self
    }
}

impl PromptSection for AgentProfilePromptSection {
    fn name(&self) -> &str {
        "agent_profile"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        Ok(render_agent_profile_block(
            &self.body,
            self.workspace_notice.as_deref(),
        ))
    }
}

/// The `## Agent profile` block as text — what [`AgentProfilePromptSection`]
/// renders, exposed so a prompt path without a [`PromptContext`] (the native
/// channel runtime) emits byte-identical output. Empty when both inputs are
/// blank.
pub fn render_agent_profile_block(body: &str, workspace_notice: Option<&str>) -> String {
    let body = body.trim();
    let notice = workspace_notice.map(str::trim).unwrap_or_default();
    let mut parts: Vec<&str> = Vec::new();
    if !body.is_empty() {
        parts.push(body);
    }
    if !notice.is_empty() {
        parts.push(notice);
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("## Agent profile\n\n{}", parts.join("\n\n"))
}

#[cfg(test)]
#[path = "prompt_section_tests.rs"]
mod tests;
