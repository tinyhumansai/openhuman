//! MCP Resources surface — `resources/list` + `resources/read`.
//!
//! Exposes OpenHuman's bundled prompt assets — the three core identity
//! files (`IDENTITY.md`, `SOUL.md`, `USER.md`) plus each built-in
//! subagent's static `prompt.md` — as MCP resources so external MCP
//! clients (Claude Desktop, Cursor, …) can attach them as conversation
//! context.
//!
//! All resources are static: their content is `include_str!`-bundled
//! into the binary at compile time, so the surface has no async I/O,
//! no permission gating, and no dynamic configuration to load. The
//! `BUILTINS` slice in `agent::agents::loader` is the source of truth
//! for which subagents ship; if a new agent is added there with a
//! `prompt.md`, add a matching entry here so it shows up over MCP.
//!
//! Spec reference: <https://modelcontextprotocol.io/specification/2025-06-18/server/resources>.

use serde_json::{json, Value};

/// A single bundled resource — static, read-only, no async work.
struct StaticResource {
    /// MCP resource URI. Stable across releases; clients store this.
    uri: &'static str,
    /// Machine-readable name (per MCP spec: required, kebab-case).
    name: &'static str,
    /// Human-readable label surfaced in client UIs (Claude Desktop, …).
    title: &'static str,
    /// One-line description shown next to the title in client UIs.
    description: &'static str,
    /// MIME type of the resource body. All bundled assets are markdown.
    mime_type: &'static str,
    /// Resource body — `include_str!`-bundled, no I/O at request time.
    content: &'static str,
}

/// Every static MCP resource the server advertises. The `include_str!`
/// calls double as a compile-time check that the asset paths still
/// exist after any agent/prompt file rename.
const RESOURCES: &[StaticResource] = &[
    StaticResource {
        uri: "openhuman://core/identity",
        name: "core-identity",
        title: "OpenHuman core identity",
        description: "Top-level identity scaffold shared by every OpenHuman subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/prompts/IDENTITY.md"),
    },
    StaticResource {
        uri: "openhuman://core/soul",
        name: "core-soul",
        title: "OpenHuman core soul",
        description: "Voice, tone, and behavioural posture shared by every OpenHuman subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/prompts/SOUL.md"),
    },
    StaticResource {
        uri: "openhuman://core/user",
        name: "core-user",
        title: "OpenHuman user-prompt scaffold",
        description: "Default user-message envelope shared by every OpenHuman subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/prompts/USER.md"),
    },
    StaticResource {
        uri: "openhuman://agents/archivist/prompt",
        name: "agent-archivist-prompt",
        title: "Archivist subagent prompt",
        description: "Static prompt body for the `archivist` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/archivist/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/code_executor/prompt",
        name: "agent-code-executor-prompt",
        title: "Code-executor subagent prompt",
        description: "Static prompt body for the `code_executor` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/code_executor/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/critic/prompt",
        name: "agent-critic-prompt",
        title: "Critic subagent prompt",
        description: "Static prompt body for the `critic` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/critic/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/crypto_agent/prompt",
        name: "agent-crypto-agent-prompt",
        title: "Crypto-agent subagent prompt",
        description: "Static prompt body for the `crypto_agent` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/crypto_agent/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/help/prompt",
        name: "agent-help-prompt",
        title: "Help subagent prompt",
        description: "Static prompt body for the `help` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/help/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/integrations_agent/prompt",
        name: "agent-integrations-agent-prompt",
        title: "Integrations-agent subagent prompt",
        description: "Static prompt body for the `integrations_agent` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/integrations_agent/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/markets_agent/prompt",
        name: "agent-markets-agent-prompt",
        title: "Markets-agent subagent prompt",
        description: "Static prompt body for the `markets_agent` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/markets_agent/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/mcp_setup/prompt",
        name: "agent-mcp-setup-prompt",
        title: "MCP-setup subagent prompt",
        description: "Static prompt body for the `mcp_setup` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/mcp_setup/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/morning_briefing/prompt",
        name: "agent-morning-briefing-prompt",
        title: "Morning-briefing subagent prompt",
        description: "Static prompt body for the `morning_briefing` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/morning_briefing/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/orchestrator/prompt",
        name: "agent-orchestrator-prompt",
        title: "Orchestrator subagent prompt",
        description: "Static prompt body for the `orchestrator` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/orchestrator/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/planner/prompt",
        name: "agent-planner-prompt",
        title: "Planner subagent prompt",
        description: "Static prompt body for the `planner` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/planner/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/researcher/prompt",
        name: "agent-researcher-prompt",
        title: "Researcher subagent prompt",
        description: "Static prompt body for the `researcher` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/researcher/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/skill_creator/prompt",
        name: "agent-skill-creator-prompt",
        title: "Skill-creator subagent prompt",
        description: "Static prompt body for the `skill_creator` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/skill_creator/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/summarizer/prompt",
        name: "agent-summarizer-prompt",
        title: "Summarizer subagent prompt",
        description: "Static prompt body for the `summarizer` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/summarizer/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/tool_maker/prompt",
        name: "agent-tool-maker-prompt",
        title: "Tool-maker subagent prompt",
        description: "Static prompt body for the `tool_maker` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/tool_maker/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/tools_agent/prompt",
        name: "agent-tools-agent-prompt",
        title: "Tools-agent subagent prompt",
        description: "Static prompt body for the `tools_agent` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/tools_agent/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/trigger_reactor/prompt",
        name: "agent-trigger-reactor-prompt",
        title: "Trigger-reactor subagent prompt",
        description: "Static prompt body for the `trigger_reactor` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/trigger_reactor/prompt.md"),
    },
    StaticResource {
        uri: "openhuman://agents/trigger_triage/prompt",
        name: "agent-trigger-triage-prompt",
        title: "Trigger-triage subagent prompt",
        description: "Static prompt body for the `trigger_triage` built-in subagent.",
        mime_type: "text/markdown",
        content: include_str!("../agent/agents/trigger_triage/prompt.md"),
    },
];

/// Build the `resources/list` response body.
///
/// `cursor` is accepted for spec-compliance but ignored: the catalog is
/// small (≈ 20 entries) so the entire list fits in one page, and the
/// response omits `nextCursor` accordingly.
pub fn list_resources_result(_cursor: Option<&str>) -> Value {
    let resources: Vec<Value> = RESOURCES
        .iter()
        .map(|r| {
            json!({
                "uri": r.uri,
                "name": r.name,
                "title": r.title,
                "description": r.description,
                "mimeType": r.mime_type,
            })
        })
        .collect();
    json!({ "resources": resources })
}

/// Build the `resources/read` response body for a single URI.
///
/// Returns `ResourceError::NotFound` if no static catalog entry matches
/// — the caller maps this to JSON-RPC `-32002` per the MCP error code
/// convention for resource lookups.
pub fn read_resource_result(uri: &str) -> Result<Value, ResourceError> {
    let resource = RESOURCES
        .iter()
        .find(|r| r.uri == uri)
        .ok_or_else(|| ResourceError::NotFound(uri.to_string()))?;
    Ok(json!({
        "contents": [
            {
                "uri": resource.uri,
                "mimeType": resource.mime_type,
                "text": resource.content,
            }
        ]
    }))
}

/// Lookup failure for `resources/read`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceError {
    /// No entry in the static catalog matches the requested URI.
    NotFound(String),
}

impl ResourceError {
    pub fn message(&self) -> String {
        match self {
            Self::NotFound(uri) => format!("unknown MCP resource `{uri}`"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn list_resources_advertises_core_identity_triple() {
        let out = list_resources_result(None);
        let resources = out
            .get("resources")
            .and_then(Value::as_array)
            .expect("resources array");
        let uris: Vec<&str> = resources
            .iter()
            .filter_map(|r| r.get("uri").and_then(Value::as_str))
            .collect();
        assert!(uris.contains(&"openhuman://core/identity"));
        assert!(uris.contains(&"openhuman://core/soul"));
        assert!(uris.contains(&"openhuman://core/user"));
    }

    #[test]
    fn list_resources_advertises_every_subagent_prompt() {
        // Locks the catalog against silent drift in both directions:
        // a new subagent added to `agent::agents::loader::BUILTINS` without a
        // matching catalog entry, OR a catalog entry that has fallen out of
        // `BUILTINS`, both fail this exact-set assertion. The list below
        // mirrors `BUILTINS` (sorted).
        let expected: BTreeSet<&str> = [
            "openhuman://agents/archivist/prompt",
            "openhuman://agents/code_executor/prompt",
            "openhuman://agents/critic/prompt",
            "openhuman://agents/crypto_agent/prompt",
            "openhuman://agents/help/prompt",
            "openhuman://agents/integrations_agent/prompt",
            "openhuman://agents/markets_agent/prompt",
            "openhuman://agents/mcp_setup/prompt",
            "openhuman://agents/morning_briefing/prompt",
            "openhuman://agents/orchestrator/prompt",
            "openhuman://agents/planner/prompt",
            "openhuman://agents/researcher/prompt",
            "openhuman://agents/skill_creator/prompt",
            "openhuman://agents/summarizer/prompt",
            "openhuman://agents/tool_maker/prompt",
            "openhuman://agents/tools_agent/prompt",
            "openhuman://agents/trigger_reactor/prompt",
            "openhuman://agents/trigger_triage/prompt",
        ]
        .into_iter()
        .collect();
        let out = list_resources_result(None);
        let resources = out
            .get("resources")
            .and_then(Value::as_array)
            .expect("resources array");
        let actual: BTreeSet<&str> = resources
            .iter()
            .filter_map(|r| r.get("uri").and_then(Value::as_str))
            .filter(|uri| uri.starts_with("openhuman://agents/"))
            .collect();
        assert_eq!(
            actual, expected,
            "subagent resource catalog drift vs `agent::agents::loader::BUILTINS`"
        );
    }

    #[test]
    fn list_resources_entries_carry_required_fields() {
        let out = list_resources_result(None);
        let resources = out
            .get("resources")
            .and_then(Value::as_array)
            .expect("resources array");
        for r in resources {
            for key in ["uri", "name", "title", "description", "mimeType"] {
                assert!(
                    r.get(key).and_then(Value::as_str).is_some(),
                    "resource entry missing `{key}`: {r}"
                );
            }
        }
    }

    #[test]
    fn list_resources_omits_next_cursor_for_single_page() {
        // Acceptance criterion for the MVP: the catalog is small enough
        // that the response is a single page. We do not yet emit
        // `nextCursor`, and clients should treat its absence as "end of
        // list" per the MCP spec.
        let out = list_resources_result(None);
        assert!(out.get("nextCursor").is_none());
    }

    #[test]
    fn read_resource_returns_text_content_for_known_uri() {
        let out =
            read_resource_result("openhuman://core/identity").expect("core identity must resolve");
        let contents = out
            .get("contents")
            .and_then(Value::as_array)
            .expect("contents array");
        assert_eq!(contents.len(), 1);
        let entry = &contents[0];
        assert_eq!(
            entry.get("uri").and_then(Value::as_str),
            Some("openhuman://core/identity")
        );
        assert_eq!(
            entry.get("mimeType").and_then(Value::as_str),
            Some("text/markdown")
        );
        let text = entry
            .get("text")
            .and_then(Value::as_str)
            .expect("text body");
        assert!(!text.is_empty(), "core identity body must not be empty");
    }

    #[test]
    fn read_resource_returns_not_found_for_unknown_uri() {
        let err =
            read_resource_result("openhuman://does-not-exist").expect_err("unknown URI must error");
        assert_eq!(
            err,
            ResourceError::NotFound("openhuman://does-not-exist".to_string())
        );
        assert!(err.message().contains("unknown MCP resource"));
    }

    #[test]
    fn read_resource_returns_distinct_content_per_uri() {
        // Defends against an accidental copy/paste where two catalog
        // entries point at the same `include_str!`.
        let a = read_resource_result("openhuman://agents/orchestrator/prompt")
            .expect("orchestrator")
            .get("contents")
            .and_then(Value::as_array)
            .and_then(|c| c.first().cloned())
            .and_then(|e| e.get("text").and_then(Value::as_str).map(str::to_string))
            .expect("orchestrator text");
        let b = read_resource_result("openhuman://agents/researcher/prompt")
            .expect("researcher")
            .get("contents")
            .and_then(Value::as_array)
            .and_then(|c| c.first().cloned())
            .and_then(|e| e.get("text").and_then(Value::as_str).map(str::to_string))
            .expect("researcher text");
        assert_ne!(
            a, b,
            "orchestrator and researcher prompts must not share bundled content"
        );
    }
}
