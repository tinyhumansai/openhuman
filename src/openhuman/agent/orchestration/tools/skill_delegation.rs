//! Single collapsed delegation tool for Composio-backed integrations
//! (#1335).
//!
//! Replaces the previous per-toolkit fan-out where the orchestrator's
//! function-calling schema gained a new `delegate_<toolkit>` entry for
//! every connected integration. Every one of those tools dispatched to
//! the same `integrations_agent` with a different `skill_filter`, so
//! exposing them separately bloated the orchestrator's tool list
//! linearly with no behavioural benefit.
//!
//! The collapsed tool keeps the routing handle the orchestrator needs
//! ("send this to integrations, scoped to toolkit X") while making the
//! orchestrator's schema cost constant in the integration dimension.
//!
//! The list of connected toolkits is rendered inline in the tool
//! description so the orchestrator still discovers which integrations
//! are available without each one being its own schema entry.

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::tools::orchestrator_tools::sanitise_slug;
use crate::openhuman::tools::traits::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolResult,
};
use tinytools::ToolRunContext;

/// Canonical tool name surfaced to the orchestrator LLM.
pub const INTEGRATIONS_DELEGATE_TOOL_NAME: &str = "delegate_to_integrations_agent";

/// Single collapsed delegation tool for all connected Composio toolkits.
///
/// Carries the slugs + one-line descriptions of every connected toolkit
/// so the tool's `description()` (which is what the orchestrator's LLM
/// sees) enumerates the routing choices without needing N tools to
/// represent them.
pub struct SkillDelegationTool {
    pub tool_name: String,
    /// `(slug, description)` for every currently-connected toolkit.
    /// `slug` is already `sanitise_slug`'d so it can be matched against
    /// the LLM-provided `toolkit` argument with a plain `==`.
    pub connected_toolkits: Vec<(String, String)>,
    pub tool_description: String,
}

impl SkillDelegationTool {
    /// Build the canonical collapsed tool from the connected-toolkit
    /// list. Returns `None` when there are zero connected toolkits —
    /// callers in `collect_orchestrator_tools` interpret that as "don't
    /// expose any integrations delegation surface at all", which is the
    /// right thing to do because the orchestrator can't usefully route
    /// to an empty set.
    pub fn for_connected(connected: Vec<(String, String)>) -> Option<Self> {
        if connected.is_empty() {
            return None;
        }
        let description = build_description(&connected);
        Some(Self {
            tool_name: INTEGRATIONS_DELEGATE_TOOL_NAME.to_string(),
            connected_toolkits: connected,
            tool_description: description,
        })
    }
}

fn build_description(connected: &[(String, String)]) -> String {
    let mut buf = String::from(
        "Use only when direct response/direct tools are insufficient and the task truly \
         requires external integration actions. Routes the work to the integrations_agent \
         with the named toolkit pre-selected. Required argument `toolkit` must be one of \
         the currently-connected slugs below; pass the user's task verbatim as `prompt`. \
         Connected toolkits:",
    );
    for (slug, desc) in connected {
        buf.push_str("\n - ");
        buf.push_str(slug);
        let trimmed = desc.trim();
        if !trimmed.is_empty() {
            buf.push_str(": ");
            buf.push_str(trimmed);
        }
    }
    buf
}

// Test-only override for the live status fetch. When set, the live re-check
// returns this value instead of touching `Config::load_or_init` /
// `fetch_connected_integrations_status`, which would otherwise read the host
// machine's login/config state and could hit the Composio backend over HTTP.
// `Some(None)` forces the "Unavailable" outcome (no live data);
// `Some(Some(vec))` injects a deterministic connected set.
#[cfg(test)]
thread_local! {
    static LIVE_FETCH_OVERRIDE: std::cell::RefCell<Option<Option<Vec<String>>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn set_live_fetch_override(value: Option<Vec<String>>) {
    LIVE_FETCH_OVERRIDE.with(|o| *o.borrow_mut() = Some(value));
}

#[cfg(test)]
fn clear_live_fetch_override() {
    LIVE_FETCH_OVERRIDE.with(|o| *o.borrow_mut() = None);
}

async fn fetch_live_connected_toolkit_slugs_once() -> Option<Vec<String>> {
    #[cfg(test)]
    {
        if let Some(injected) = LIVE_FETCH_OVERRIDE.with(|o| o.borrow().clone()) {
            return injected;
        }
    }
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .ok()?;
    match crate::openhuman::integrations::composio::fetch_connected_integrations_status(&config).await {
        crate::openhuman::integrations::composio::FetchConnectedIntegrationsStatus::Authoritative(entries) => {
            let mut toolkits: Vec<String> = entries
                .into_iter()
                .filter(|entry| entry.connected)
                .map(|entry| sanitise_slug(&entry.toolkit))
                .collect();
            toolkits.sort();
            toolkits.dedup();
            Some(toolkits)
        }
        crate::openhuman::integrations::composio::FetchConnectedIntegrationsStatus::Unavailable => None,
    }
}

fn resolve_connected_toolkits(
    snapshot: &[(String, String)],
    slug: &str,
    live_connected: Option<&[String]>,
) -> (bool, Vec<String>) {
    let allowed: Vec<String> = snapshot.iter().map(|(slug, _)| slug.clone()).collect();
    if snapshot.iter().any(|(known_slug, _)| known_slug == slug) {
        return (true, allowed);
    }
    if let Some(live) = live_connected {
        if live.iter().any(|s| s == slug) {
            return (true, live.to_vec());
        }
    }
    (false, allowed)
}

#[async_trait]
impl Tool for SkillDelegationTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let slugs: Vec<&str> = self
            .connected_toolkits
            .iter()
            .map(|(slug, _)| slug.as_str())
            .collect();
        json!({
            "type": "object",
            "required": ["toolkit", "prompt"],
            "properties": {
                "toolkit": {
                    "type": "string",
                    "enum": slugs,
                    "description": "Composio toolkit slug to route to (e.g. `gmail`, `notion`). \
                                    Must match one of the connected toolkits enumerated in this tool's description."
                },
                // `prompt` and `model` are described once in the parent's
                // prompt.md ("Structured handoffs") rather than here, matching
                // `ArchetypeDelegationTool`. `toolkit` keeps its description:
                // it is the routing signal and points at the slugs enumerated
                // in this tool's own description.
                "prompt": { "type": "string" },
                "model": {
                    "type": "string",
                    "description": "Pin the child to this exact model id. Omit unless you have a reason."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let raw_toolkit = args
            .get("toolkit")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        log::debug!(
            "[skill-delegation] execute start tool='{}' raw_toolkit={:?} prompt_chars={}",
            self.tool_name,
            raw_toolkit,
            args.get("prompt")
                .and_then(|v| v.as_str())
                .map(|s| s.chars().count())
                .unwrap_or(0)
        );
        if raw_toolkit.is_empty() {
            log::debug!(
                "[skill-delegation] reject: missing `toolkit` argument for tool='{}'",
                self.tool_name
            );
            return Ok(ToolResult::error(format!(
                "{}: `toolkit` is required and must match a connected integration slug",
                self.tool_name
            )));
        }
        let slug = sanitise_slug(&raw_toolkit);
        let mut live_connected: Option<Vec<String>> = None;
        let mut known = self
            .connected_toolkits
            .iter()
            .any(|(known_slug, _)| known_slug == &slug);
        if !known {
            // Safety net for same-thread OAuth races: do one live status
            // refresh before rejecting an unknown toolkit, mirroring the
            // spawn_subagent integrations pre-flight.
            live_connected = fetch_live_connected_toolkit_slugs_once().await;
        }
        let (known_after_recheck, allowed) =
            resolve_connected_toolkits(&self.connected_toolkits, &slug, live_connected.as_deref());
        if known_after_recheck && !known {
            log::info!(
                "[skill-delegation] toolkit '{}' accepted after live re-check (session schema stale)",
                slug
            );
        }
        known = known_after_recheck;
        if !known {
            log::debug!(
                "[skill-delegation] reject: toolkit '{}' (sanitised='{}') not in connected set {:?}",
                raw_toolkit,
                slug,
                allowed
            );
            return Ok(ToolResult::error(format!(
                "{}: toolkit `{raw_toolkit}` is not connected — allowed: [{}]",
                self.tool_name,
                allowed.join(", ")
            )));
        }

        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if prompt.is_empty() {
            log::debug!(
                "[skill-delegation] reject: empty `prompt` for tool='{}' toolkit='{}'",
                self.tool_name,
                slug
            );
            return Ok(ToolResult::error(format!(
                "{}: `prompt` is required",
                self.tool_name
            )));
        }

        let model_override = args
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        log::debug!(
            "[skill-delegation] dispatching toolkit='{}' to integrations_agent (prompt_chars={})",
            slug,
            prompt.chars().count()
        );
        // Integration delegations stay blocking: their outcomes (send the
        // email, create the page, …) are usually approval-gated mid-turn and
        // the orchestrator's reply reports the concrete result. The durable
        // async default applies to archetype delegations only for now.
        super::dispatch_subagent(
            "integrations_agent",
            &self.tool_name,
            &prompt,
            Some(&slug),
            model_override,
            tool_context,
            super::dispatch::DispatchMode::Blocking,
        )
        .await
    }
}

#[cfg(test)]
#[path = "skill_delegation_tests.rs"]
mod tests;
