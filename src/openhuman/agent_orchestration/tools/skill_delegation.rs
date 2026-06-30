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
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};

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
    match crate::openhuman::composio::fetch_connected_integrations_status(&config).await {
        crate::openhuman::composio::FetchConnectedIntegrationsStatus::Authoritative(entries) => {
            let mut toolkits: Vec<String> = entries
                .into_iter()
                .filter(|entry| entry.connected)
                .map(|entry| sanitise_slug(&entry.toolkit))
                .collect();
            toolkits.sort();
            toolkits.dedup();
            Some(toolkits)
        }
        crate::openhuman::composio::FetchConnectedIntegrationsStatus::Unavailable => None,
    }
}

fn resolve_connected_toolkits(
    snapshot: &[(String, String)],
    slug: &str,
    live_connected: Option<&[String]>,
) -> (bool, Vec<String>) {
    let mut allowed: Vec<String> = snapshot.iter().map(|(slug, _)| slug.clone()).collect();
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
                "prompt": {
                    "type": "string",
                    "description": "Clear instruction for what to do. Include all relevant context — the sub-agent has no memory of your conversation."
                },
                "model": {
                    "type": "string",
                    "description": "Optional exact model id for this delegation only. Keeps the parent provider/routing, but pins the child agent to this model instead of the agent definition's default."
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
        super::dispatch_subagent(
            "integrations_agent",
            &self.tool_name,
            &prompt,
            Some(&slug),
            model_override,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_connected_returns_none_when_no_toolkits() {
        assert!(SkillDelegationTool::for_connected(vec![]).is_none());
    }

    #[test]
    fn for_connected_uses_canonical_tool_name() {
        let tool = SkillDelegationTool::for_connected(vec![(
            "gmail".to_string(),
            "Email access.".to_string(),
        )])
        .unwrap();
        assert_eq!(tool.name(), INTEGRATIONS_DELEGATE_TOOL_NAME);
        assert_eq!(tool.name(), "delegate_to_integrations_agent");
    }

    #[test]
    fn description_enumerates_connected_toolkits() {
        let tool = SkillDelegationTool::for_connected(vec![
            ("gmail".to_string(), "Email access.".to_string()),
            ("notion".to_string(), "Pages and databases.".to_string()),
        ])
        .unwrap();
        let desc = tool.description();
        assert!(desc.contains("gmail"));
        assert!(desc.contains("notion"));
        assert!(desc.contains("Email access."));
        assert!(desc.contains("Pages and databases."));
    }

    #[test]
    fn parameters_schema_enforces_toolkit_enum_against_connected_slugs() {
        let tool = SkillDelegationTool::for_connected(vec![
            ("gmail".to_string(), "Email.".to_string()),
            ("notion".to_string(), "Docs.".to_string()),
        ])
        .unwrap();
        let schema = tool.parameters_schema();
        let enum_vals = schema["properties"]["toolkit"]["enum"]
            .as_array()
            .expect("toolkit enum is an array");
        let collected: Vec<&str> = enum_vals.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(collected, vec!["gmail", "notion"]);

        let required = schema["required"].as_array().expect("required is an array");
        let required: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required.contains(&"toolkit"));
        assert!(required.contains(&"prompt"));
    }

    #[tokio::test]
    async fn execute_rejects_missing_toolkit_argument() {
        let tool =
            SkillDelegationTool::for_connected(vec![("gmail".to_string(), "Email.".to_string())])
                .unwrap();
        let result = tool.execute(json!({"prompt": "x"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("toolkit"));
    }

    #[tokio::test]
    async fn execute_rejects_unknown_toolkit_with_allowed_list() {
        // Force the live re-check to return "Unavailable" so the test never
        // reads host config or reaches the Composio backend — the reject must
        // come purely from the in-memory snapshot (gmail/notion, no slack).
        set_live_fetch_override(None);
        let tool = SkillDelegationTool::for_connected(vec![
            ("gmail".to_string(), "Email.".to_string()),
            ("notion".to_string(), "Docs.".to_string()),
        ])
        .unwrap();
        let result = tool
            .execute(json!({"toolkit": "slack", "prompt": "hi"}))
            .await
            .unwrap();
        clear_live_fetch_override();
        assert!(result.is_error);
        let body = result.output();
        assert!(body.contains("slack"));
        assert!(body.contains("gmail"));
        assert!(body.contains("notion"));
    }

    #[tokio::test]
    async fn execute_rejects_empty_prompt() {
        let tool =
            SkillDelegationTool::for_connected(vec![("gmail".to_string(), "Email.".to_string())])
                .unwrap();
        let result = tool
            .execute(json!({"toolkit": "gmail", "prompt": "   "}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("prompt"));
    }

    #[tokio::test]
    async fn execute_normalises_toolkit_input_before_matching() {
        // Mixed-case + odd-character user input must collapse onto the
        // canonical slug before the connectedness check fires.
        // Pin the live re-check to the same snapshot so the test is hermetic
        // (no host config / backend read): `gmail` stays unknown, while the
        // normalised `google_calendar` is accepted.
        set_live_fetch_override(Some(vec!["google_calendar".to_string()]));
        let tool = SkillDelegationTool::for_connected(vec![(
            "google_calendar".to_string(),
            "Calendar.".to_string(),
        )])
        .unwrap();
        // "GMail" sanitises to `gmail` — NOT in the connected set, so it
        // must be rejected with the unknown-toolkit message that
        // enumerates the allowed slugs.
        let bad = tool
            .execute(json!({"toolkit": "GMail", "prompt": "x"}))
            .await
            .unwrap();
        assert!(bad.is_error);
        let bad_body = bad.output();
        assert!(
            bad_body.contains("not connected"),
            "expected unknown-toolkit error path, got: {bad_body}"
        );
        assert!(bad_body.contains("google_calendar"));

        // "Google-Calendar" sanitises to `google_calendar`, which IS in
        // the connected set, so the toolkit gate must let it through.
        // Dispatch will then fail because no agent registry is wired up
        // in this unit-test process — but the error must NOT be the
        // unknown-toolkit branch, because that branch was supposed to
        // be bypassed by the slug normalisation.
        let ok = tool
            .execute(json!({"toolkit": "Google-Calendar", "prompt": "do thing"}))
            .await;
        match ok {
            Ok(result) => {
                let body = result.output();
                assert!(
                    !body.contains("not connected"),
                    "normalised slug should pass the toolkit gate, got: {body}"
                );
            }
            Err(err) => {
                let msg = err.to_string();
                assert!(
                    !msg.contains("not connected"),
                    "normalised slug should pass the toolkit gate, got: {msg}"
                );
            }
        }
        clear_live_fetch_override();
    }

    #[test]
    fn resolve_connected_toolkits_prefers_live_recheck_for_unknown_slug() {
        let snapshot = vec![("gmail".to_string(), "Email".to_string())];

        let (known_snapshot, allowed_snapshot) =
            resolve_connected_toolkits(&snapshot, "gmail", None);
        assert!(known_snapshot);
        assert_eq!(allowed_snapshot, vec!["gmail".to_string()]);

        let live = vec!["gmail".to_string(), "notion".to_string()];
        let (known_live, allowed_live) =
            resolve_connected_toolkits(&snapshot, "notion", Some(live.as_slice()));
        assert!(known_live);
        assert_eq!(allowed_live, live);

        let live_no_match = vec!["gmail".to_string(), "notion".to_string()];
        let (known_none, allowed_none) =
            resolve_connected_toolkits(&snapshot, "slack", Some(live_no_match.as_slice()));
        assert!(!known_none);
        assert_eq!(allowed_none, vec!["gmail".to_string()]);
    }
}
