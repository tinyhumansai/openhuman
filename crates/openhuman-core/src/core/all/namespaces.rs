//! Human-readable descriptions of the RPC namespaces, for CLI help output.

use super::extension_namespace_description;

/// Returns a human-readable description for a given namespace.
///
/// This is used for CLI help output.
pub fn namespace_description(namespace: &str) -> Option<&'static str> {
    match namespace {
        "about_app" => Some("Catalog the app's user-facing capabilities and where to find them."),
        "agent" => Some("Per-thread agent run-mode control (Plan vs Build)."),
        "ai" => Some("Agent-generated artifact storage, retrieval, and lifecycle management."),
        "app_state" => Some("Expose core-owned app shell state for frontend polling."),
        "profiles" => {
            Some("SaaS operator plane: provision and inspect the profile serving each user.")
        }
        "auth" => Some("Manage app session and provider credentials."),
        "agent_experience" => Some("Local procedural experience capture and retrieval for agents."),
        "channels" => Some("Channel definitions, connections, and lifecycle management."),
        "composio" => Some(
            "Composio OAuth integrations proxied via the backend — toolkits, connections, tools, and actions.",
        ),
        "config" => Some("Read and update persisted runtime configuration."),
        "connectivity" => Some(
            "Connectivity diagnostics for the local sidecar, listening port, and backend Socket.IO state.",
        ),
        "cron" => Some("Manage scheduled jobs and run history."),
        "inbound" => {
            Some("Inbound event queues: depths and outcome counts, dead letters and redrive.")
        }
        "flows" => Some("Create, store, and run automation workflows."),
        "dashboard" => {
            Some("Operator-facing dashboard aggregations: per-model health comparison rows.")
        }
        "mcp_clients" => Some(
            "Browse the MCP registries, declare the user's servers in one mcp.json document, manage their connections and credentials, and expose their tools to the agent.",
        ),
        "mcp_ui" => Some("Serve MCP tool widgets their documents and run their tool calls."),
        "decrypt" => Some("Decrypt secure values managed by secret storage."),
        "doctor" => Some("Run diagnostics for workspace and runtime health."),
        "hooks" => Some(
            "User-authored hooks: list what is configured, reload every hooks.json layer, and test-fire one event.",
        ),
        "encrypt" => Some("Encrypt secure values managed by secret storage."),
        "health" => Some("Process and component health snapshots."),
        "inference" => {
            Some("Connect to configured text, vision, and embedding inference runtimes.")
        }
        "javascript" => {
            Some("First-class JavaScript runtime bridge for listing and dispatching tools.")
        }
        "security" => Some("Security policy and autonomy guardrail metadata."),
        "service" => Some("Desktop service lifecycle management."),
        "session_import" => {
            Some("One-time import of legacy session transcripts into TinyAgents stores.")
        }
        "skill_registry" => Some(
            "Browse, search, install, and uninstall skills from remote registries (OpenHuman, Hermes, OpenClaw).",
        ),
        "skill_runtime" => {
            Some("Run installed skills, inspect run logs, and resolve Node/Python skill runtimes.")
        }
        "skills" => {
            Some("Discovered SKILL.md skills (discovery, parse, install, run) and their resources.")
        }
        "socket" => Some("Backend Socket.IO bridge controls."),
        "memory" => Some(
            "Memory v2: engine selection, recall, fetch, learn, forget, conversations, document sources, context.md and v1 import.",
        ),
        "run_ledger" => Some(
            "Durable agent and workflow run state, child lineage, events, telemetry, and checkpoint references.",
        ),
        "agent_work" => Some(
            "Background agent command center — recent agent runs grouped by status (needs-input, working, completed, failed, stopped).",
        ),
        "workflow_run" => Some(
            "Durable dynamic workflow runs — declarative multi-agent definitions and the read surface over persisted runs.",
        ),
        "agent_team" => Some(
            "Durable agent-team coordination: teams, members, dependency-aware task claiming, and teammate messaging.",
        ),
        "tool_registry" => Some(
            "Read-only discovery for MCP stdio tools and controller-backed tools, including routes, schemas, version, allowed agents, and health.",
        ),
        "test" => Some("E2E test support — wipe sidecar state in-place between specs."),
        "wallet" => {
            Some("Local wallet onboarding status and derived multi-chain account metadata.")
        }
        "web3_swap" => Some("Single-chain crypto swaps via deBridge, built on the local wallet."),
        "web3_bridge" => {
            Some("Cross-chain crypto bridges via deBridge DLN, built on the local wallet.")
        }
        "web3_dapp" => Some("Generic EVM dapp contract calls signed by the local wallet."),
        "provider_surfaces" => {
            Some("Local-first assistive surfaces for provider events, respond queues, and drafts.")
        }
        "voice" => Some("Speech-to-text and text-to-speech using local models."),
        "webhooks" => {
            Some("Webhook tunnel registrations and captured request/response debug logs.")
        }
        "update" => {
            Some("Self-update: check GitHub Releases for newer core binary and stage updates.")
        }

        "notification" => Some(
            "Integration notification ingest, triage scoring, listing, read-state, \
             and per-provider routing settings.",
        ),
        "devices" => Some(
            "Paired mobile device management — pairing channel creation, listing, and revocation.",
        ),
        "storage" => Some("Storage layout v1 to v2 migration, schema status and restore."),
        "subsystems" => Some(
            "Kernel subsystem slots and their bound drivers: class, health, contract version, and advertised capabilities.",
        ),
        other => extension_namespace_description(other),
    }
}
