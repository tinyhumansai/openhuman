//! Static MCP resource catalog for bundled prompt assets.
//!
//! Exposes `IDENTITY.md`, `SOUL.md`, `USER.md` and the `prompt.md` template
//! for each built-in subagent as MCP resources. The content is
//! embedded at compile time via `include_str!`.
//!
//! ## URI scheme
//!
//! | Resource            | URI                                      |
//! |---------------------|------------------------------------------|
//! | `IDENTITY.md`       | `openhuman://prompts/identity`           |
//! | `SOUL.md`           | `openhuman://prompts/soul`               |
//! | `USER.md`           | `openhuman://prompts/user`               |
//! | `<id>/prompt.md`    | `openhuman://prompts/agents/<id>`        |
//!
//! ## Catalog parity
//!
//! The unit test `catalog_mirrors_builtins` cross-references this catalog
//! against `BUILTINS` in `loader.rs`. Adding a new built-in subagent without
//! a matching catalog entry fails that test and therefore CI.

use serde_json::{json, Value};

struct PromptResource {
    uri: &'static str,
    name: &'static str,
    description: &'static str,
    content: &'static str,
}

const RESOURCE_CATALOG: &[PromptResource] = &[
    // ── Core prompts ──────────────────────────────────────────────────────
    PromptResource {
        uri: "openhuman://prompts/identity",
        name: "Agent Identity",
        description: "Core agent identity definition (IDENTITY.md).",
        content: include_str!("../agent/prompts/IDENTITY.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/soul",
        name: "Agent Soul",
        description: "Core agent personality and values (SOUL.md).",
        content: include_str!("../agent/prompts/SOUL.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/user",
        name: "User Context",
        description: "Core user-profile context injected into every session (USER.md).",
        content: include_str!("../agent/prompts/USER.md"),
    },
    // ── Subagent prompt templates ─────────────────────────────────────────
    PromptResource {
        uri: "openhuman://prompts/agents/orchestrator",
        name: "orchestrator",
        description: "Chat-tier orchestrator that routes tasks to specialist subagents.",
        content: include_str!("../agent_registry/agents/orchestrator/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/planner",
        name: "planner",
        description: "Reasoning-tier planner that grounds multi-step plans in integration data.",
        content: include_str!("../agent_registry/agents/planner/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/code_executor",
        name: "code_executor",
        description: "Sandboxed worker that writes and executes code.",
        content: include_str!("../agent_registry/agents/code_executor/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/integrations_agent",
        name: "integrations_agent",
        description: "Worker that executes Composio integration actions.",
        content: include_str!("../agent_registry/agents/integrations_agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/crypto_agent",
        name: "crypto_agent",
        description: "Specialist worker for wallet and on-chain operations.",
        content: include_str!("../agent_registry/agents/crypto_agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/markets_agent",
        name: "markets_agent",
        description: "Specialist worker for prediction-market venues (Polymarket, Kalshi).",
        content: include_str!("../agent_registry/agents/markets_agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/tinyplace_agent",
        name: "tinyplace_agent",
        description: "Specialist worker for tiny.place identity, marketplace, messaging, and jobs.",
        content: include_str!("../tinyplace/agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/tools_agent",
        name: "tools_agent",
        description: "Generalist worker with access to the full tool surface.",
        content: include_str!("../agent_registry/agents/tools_agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/tool_maker",
        name: "tool_maker",
        description: "Sandboxed worker that creates new tools from descriptions.",
        content: include_str!("../agent_registry/agents/tool_maker/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/skill_creator",
        name: "skill_creator",
        description: "Sandboxed worker that authors and publishes skill packages.",
        content: include_str!("../agent_registry/agents/skill_creator/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/researcher",
        name: "researcher",
        description: "Worker that searches the web and synthesises research findings.",
        content: include_str!("../agent_registry/agents/researcher/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/critic",
        name: "critic",
        description: "Read-only worker that critiques plans and outputs.",
        content: include_str!("../agent_registry/agents/critic/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/vision_agent",
        name: "vision_agent",
        description: "Multimodal worker that analyses attached images for the vision tier.",
        content: include_str!("../agent_registry/agents/vision_agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/archivist",
        name: "archivist",
        description: "Background worker that distils conversations into persistent memory.",
        content: include_str!("../agent_registry/agents/archivist/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/trigger_triage",
        name: "trigger_triage",
        description: "Read-only worker that classifies incoming automation triggers.",
        content: include_str!("../agent_registry/agents/trigger_triage/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/trigger_reactor",
        name: "trigger_reactor",
        description: "Worker that executes actions in response to classified triggers.",
        content: include_str!("../agent_registry/agents/trigger_reactor/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/morning_briefing",
        name: "morning_briefing",
        description: "Read-only worker that assembles a personalised morning briefing.",
        content: include_str!("../agent_registry/agents/morning_briefing/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/summarizer",
        name: "summarizer",
        description: "Worker that condenses long documents or conversations.",
        content: include_str!("../agent_registry/agents/summarizer/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/help",
        name: "help",
        description: "Read-only worker that answers questions from documentation.",
        content: include_str!("../agent_registry/agents/help/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/scheduler_agent",
        name: "scheduler_agent",
        description: "Specialist worker for reminders, recurring jobs, and cron inspection.",
        content: include_str!("../agent_registry/agents/scheduler_agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/presentation_agent",
        name: "presentation_agent",
        description: "Specialist worker for evidence-grounded presentation generation.",
        content: include_str!("../agent_registry/agents/presentation_agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/desktop_control_agent",
        name: "desktop_control_agent",
        description: "Specialist worker for desktop app launch and accessibility actions.",
        content: include_str!("../agent_registry/agents/desktop_control_agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/mcp_setup",
        name: "mcp_setup",
        description: "Worker that guides the user through MCP client configuration.",
        content: include_str!("../agent_registry/agents/mcp_setup/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/mcp_agent",
        name: "mcp_agent",
        description: "Worker that discovers and calls tools on already-connected MCP servers.",
        content: include_str!("../agent_registry/agents/mcp_agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/task_manager_agent",
        name: "task_manager_agent",
        description: "Specialist worker for task planning, status, and task-board changes.",
        content: include_str!("../agent_registry/agents/task_manager_agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/settings_agent",
        name: "settings_agent",
        description: "Specialist worker for inspecting and updating OpenHuman settings.",
        content: include_str!("../agent_registry/agents/settings_agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/profile_memory_agent",
        name: "profile_memory_agent",
        description: "Specialist worker for profile and long-term memory updates.",
        content: include_str!("../agent_registry/agents/profile_memory_agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/account_admin_agent",
        name: "account_admin_agent",
        description: "Specialist worker for connected account and integration administration.",
        content: include_str!("../agent_registry/agents/account_admin_agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/screen_awareness_agent",
        name: "screen_awareness_agent",
        description: "Specialist worker for screen context and desktop state inspection.",
        content: include_str!("../agent_registry/agents/screen_awareness_agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/agent_memory",
        name: "agent_memory",
        description: "Dedicated memory retrieval subagent using smart-walk strategies.",
        content: include_str!("../agent_memory/agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/subconscious",
        name: "subconscious",
        description: "Background reasoning agent that maintains subconscious scratchpad context.",
        content: include_str!("../subconscious/agent/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/skill_setup",
        name: "skill_setup",
        description: "Worker that guides skill installation and backend configuration.",
        content: include_str!("../skill_registry/agent/skill_setup/prompt.md"),
    },
    PromptResource {
        uri: "openhuman://prompts/agents/skill_executor",
        name: "skill_executor",
        description: "Sandboxed worker that runs installed skill packages.",
        content: include_str!("../skill_runtime/agent/skill_executor/prompt.md"),
    },
];

/// Returns the `resources/list` result payload listing every catalog entry.
pub fn list_resources_result() -> Value {
    let resources: Vec<Value> = RESOURCE_CATALOG
        .iter()
        .map(|r| {
            json!({
                "uri": r.uri,
                "name": r.name,
                "description": r.description,
                "mimeType": "text/markdown"
            })
        })
        .collect();
    log::debug!("[mcp_server] resources/list count={}", resources.len());
    json!({ "resources": resources })
}

/// Returns the `resources/templates/list` result payload.
///
/// The catalog is fully static — every URI is concrete, none are templated —
/// so the response is always an empty `resourceTemplates` array. The handler
/// exists so MCP clients that probe `resources/templates/list` after seeing
/// the `resources` capability get a well-formed result instead of
/// `-32601 Method not found`.
pub fn list_resource_templates_result() -> Value {
    log::debug!("[mcp_server] resources/templates/list count=0 (catalog is static)");
    json!({ "resourceTemplates": [] })
}

/// Returns the `resources/read` result payload for the given URI, or a JSON-RPC
/// error value when the URI is unknown (`-32002`) or missing (`-32602`).
pub fn read_resource_result(params: &Value) -> Result<Value, (i64, &'static str, String)> {
    let uri = params
        .as_object()
        .and_then(|obj| obj.get("uri"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|uri| !uri.is_empty())
        .ok_or_else(|| {
            (
                -32602_i64,
                "Invalid params",
                "resources/read params.uri must be a non-empty string".to_string(),
            )
        })?;

    let resource = RESOURCE_CATALOG
        .iter()
        .find(|r| r.uri == uri)
        .ok_or_else(|| {
            log::debug!("[mcp_server] resources/read unknown uri={uri}");
            (
                -32002_i64,
                "Resource not found",
                format!("no resource with uri `{uri}`"),
            )
        })?;

    log::debug!(
        "[mcp_server] resources/read uri={uri} bytes={}",
        resource.content.len()
    );

    Ok(json!({
        "contents": [{
            "uri": resource.uri,
            "mimeType": "text/markdown",
            "text": resource.content
        }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_mirrors_builtins() {
        use crate::openhuman::agent_registry::agents::BUILTINS;

        for b in BUILTINS {
            let expected_uri = format!("openhuman://prompts/agents/{}", b.id);
            assert!(
                RESOURCE_CATALOG.iter().any(|r| r.uri == expected_uri),
                "RESOURCE_CATALOG is missing an entry for built-in agent `{}` \
                 (expected URI `{}`). Add it to RESOURCE_CATALOG in resources.rs.",
                b.id,
                expected_uri
            );
        }

        let catalog_agent_count = RESOURCE_CATALOG
            .iter()
            .filter(|r| r.uri.starts_with("openhuman://prompts/agents/"))
            .count();
        assert_eq!(
            catalog_agent_count,
            BUILTINS.len(),
            "RESOURCE_CATALOG has {catalog_agent_count} agent entries but BUILTINS has {}. \
             Remove stale entries from RESOURCE_CATALOG.",
            BUILTINS.len()
        );
    }

    #[test]
    fn list_resources_returns_all_catalog_entries() {
        let result = list_resources_result();
        let resources = result["resources"].as_array().expect("resources array");
        assert_eq!(
            resources.len(),
            RESOURCE_CATALOG.len(),
            "resources/list count mismatch"
        );
        // Every entry has required fields
        for entry in resources {
            assert!(entry["uri"].is_string(), "uri must be string");
            assert!(entry["name"].is_string(), "name must be string");
            assert_eq!(entry["mimeType"], "text/markdown");
        }
    }

    #[test]
    fn list_resources_includes_core_and_agent_uris() {
        let result = list_resources_result();
        let uris: Vec<&str> = result["resources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["uri"].as_str().unwrap())
            .collect();
        for expected in [
            "openhuman://prompts/identity",
            "openhuman://prompts/soul",
            "openhuman://prompts/user",
            "openhuman://prompts/agents/orchestrator",
            "openhuman://prompts/agents/mcp_setup",
        ] {
            assert!(uris.contains(&expected), "missing URI {expected}");
        }
    }

    #[test]
    fn read_resource_returns_content_for_known_uri() {
        let params = json!({ "uri": "openhuman://prompts/identity" });
        let result = read_resource_result(&params).expect("should succeed");
        let contents = result["contents"].as_array().expect("contents array");
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["uri"], "openhuman://prompts/identity");
        assert_eq!(contents[0]["mimeType"], "text/markdown");
        assert!(!contents[0]["text"].as_str().unwrap_or("").is_empty());
    }

    #[test]
    fn read_resource_returns_minus_32002_for_unknown_uri() {
        let params = json!({ "uri": "openhuman://prompts/agents/nonexistent" });
        let err = read_resource_result(&params).expect_err("should fail for unknown URI");
        assert_eq!(err.0, -32002);
        assert!(err.2.contains("nonexistent"));
    }

    #[test]
    fn read_resource_returns_minus_32602_for_missing_uri() {
        let params = json!({});
        let err = read_resource_result(&params).expect_err("should fail without uri");
        assert_eq!(err.0, -32602);
    }

    #[test]
    fn read_resource_returns_content_for_each_subagent() {
        use crate::openhuman::agent_registry::agents::BUILTINS;
        for b in BUILTINS {
            let uri = format!("openhuman://prompts/agents/{}", b.id);
            let params = json!({ "uri": uri });
            let result = read_resource_result(&params)
                .unwrap_or_else(|_| panic!("read_resource failed for agent `{}`", b.id));
            let text = result["contents"][0]["text"].as_str().unwrap_or("");
            assert!(
                !text.is_empty(),
                "prompt content is empty for agent `{}`",
                b.id
            );
        }
    }

    #[test]
    fn list_resource_templates_returns_empty_array() {
        let result = list_resource_templates_result();
        let templates = result["resourceTemplates"]
            .as_array()
            .expect("resourceTemplates must be a JSON array");
        assert!(
            templates.is_empty(),
            "resources/templates/list must return an empty array — the catalog is static"
        );
    }

    #[test]
    fn all_catalog_uris_are_unique() {
        let mut uris: Vec<&str> = RESOURCE_CATALOG.iter().map(|r| r.uri).collect();
        let original_len = uris.len();
        uris.sort_unstable();
        uris.dedup();
        let deduped_len = uris.len();
        assert_eq!(
            original_len, deduped_len,
            "RESOURCE_CATALOG contains duplicate URIs"
        );
    }
}
