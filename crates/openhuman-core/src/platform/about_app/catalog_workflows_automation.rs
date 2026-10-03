//! Workflows and Automation capability entries.

use super::*;

pub(super) const CAPABILITIES: &[Capability] = &[
Capability {
        id: "workflows.discover",
        name: "Discover Workflows",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Browse available workflows that can extend the app.",
        how_to: "Intelligence > Workflows",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "workflows.install",
        name: "Install Workflows",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Install a workflow into the local workspace.",
        how_to: "Intelligence > Workflows > Install",
        status: CapabilityStatus::Stable,
        // Installing fetches the skill's SKILL.md from where it is published:
        // registry skills from ClawHub or GitHub, others from the URL given.
        // The request names the skill; no user content is sent.
        privacy: Some(CapabilityPrivacy {
            leaves_device: true,
            data_kind: PrivacyDataKind::Metadata,
            destinations: &[
                "ClawHub (clawhub.ai)",
                "GitHub (raw.githubusercontent.com, api.github.com)",
                "Hermes skill catalog (hermes-agent.nousresearch.com)",
                "The host of a SKILL.md URL you install from",
            ],
        }),
    },
Capability {
        id: "workflows.configure",
        name: "Configure Workflows",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Open workflow setup and update workflow-specific configuration.",
        how_to: "Intelligence > Workflows > Setup or Connections",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "workflows.connection_status",
        name: "Monitor Workflow Connection Status",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "See whether a workflow-backed integration is connected, offline, or needs setup.",
        how_to: "Intelligence > Workflows or Connections",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
Capability {
        id: "workflows.sync_manual",
        name: "Manually Sync Workflow Data",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Trigger a manual data sync for a workflow integration.",
        how_to: "Intelligence > Workflows > Workflow card > Sync",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
Capability {
        id: "workflows.web_search",
        name: "Web Search, Answers and Page Contents",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description:
            "Search the web, get grounded answers with citations, and read page contents through several providers at once. Signed-in users get Exa and Gemini with Google Search grounding included; Brave, Tavily, Querit, Parallel, Seltz, SearXNG, and your own Exa or Gemini keys can be added. Each capability falls back to the next provider when one is unavailable.",
        how_to: "Connections > Search to choose providers and their order, then ask the assistant to look something up",
        status: CapabilityStatus::Beta,
        privacy: Some(CapabilityPrivacy {
            leaves_device: true,
            data_kind: PrivacyDataKind::Derived,
            destinations: &["OpenHuman backend (managed search)", "Your configured search providers"],
        }),
    },
Capability {
        id: "workflows.tinyfish_web_automation",
        name: "TinyFish Web Automation",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description:
            "Search the web, render JavaScript-heavy pages, and run goal-based browser automations through TinyFish.",
        how_to: "Conversations > Ask the assistant to search, fetch, or automate a website with TinyFish",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
Capability {
        id: "workflows.computer_browser_automation",
        name: "Browser Automation",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Inspect pages and run bounded browser tasks in Chrome through TinyComputer, with a rescue model for failed steps.",
        how_to: "Connections > Computer > Browser, then ask the assistant to use the browser",
        status: CapabilityStatus::Beta,
        privacy: Some(CapabilityPrivacy {
            leaves_device: true,
            data_kind: PrivacyDataKind::Raw,
            destinations: &["Visited websites", "OpenRouter, OpenHuman backend, OpenJev or Sage for task decisions"],
        }),
    },
Capability {
        id: "workflows.toggle_enabled",
        name: "Enable or Disable Workflows",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Turn individual workflows on or off without uninstalling them.",
        how_to: "Settings > Developer Options > Workflows",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "workflows.open_connections_hub",
        name: "Open Connections Hub",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Browse the dedicated connections hub for external workflow-backed integrations.",
        how_to: "Connections",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
Capability {
        id: "composio.direct_mode",
        name: "Composio Direct Mode (BYO API Key)",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description:
            "Route Composio tool calls directly to backend.composio.dev with your own API key, \
             bypassing the OpenHuman backend proxy. Tool execution only — trigger webhooks still \
             require backend mode.",
        how_to: "Settings > Skills > Composio > Direct mode",
        status: CapabilityStatus::Beta,
        privacy: COMPOSIO_DIRECT_CREDENTIALS,
    },
Capability {
        id: "composio.direct_mode_triggers_gap",
        name: "Composio Triggers (Direct Mode — Limited)",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description:
            "Composio real-time trigger webhooks (Gmail new-message, Slack new-message, …) \
             currently arrive over wss://api.tinyhumans.ai/socket.io and require backend mode. \
             Direct-mode users get synchronous tool execution but not async trigger push in \
             this release.",
        how_to: "Switch to Backend mode to receive triggers, or wait for the direct trigger sink follow-up",
        status: CapabilityStatus::ComingSoon,
        privacy: None,
    },
Capability {
        id: "workflows.connect_google",
        name: "Connect Google",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Connect Google services for email, contacts, and calendar workflows.",
        how_to: "Connections > OAuth",
        status: CapabilityStatus::ComingSoon,
        privacy: LOCAL_CREDENTIALS,
    },
Capability {
        id: "workflows.connect_notion",
        name: "Connect Notion",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Connect Notion for workspace sync and productivity workflows.",
        how_to: "Connections > OAuth",
        status: CapabilityStatus::ComingSoon,
        privacy: LOCAL_CREDENTIALS,
    },
Capability {
        id: "workflows.connect_web3_wallet",
        name: "Connect Web3 Wallet",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Set up local EVM, BTC, Solana, and Tron wallet identities from one recovery phrase.",
        how_to: "Settings > Crypto > Recovery Phrase or Connections",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_CREDENTIALS,
    },
Capability {
        id: "workflows.wallet_execution",
        name: "Wallet Execution Tools",
        domain: "wallet",
        category: CapabilityCategory::Workflows,
        description: "Read addresses and balances, prepare/confirm/execute native + token transfers (ERC20/SPL/TRC20/BEP20), and inspect transactions (status, receipt, lookup) across the connected wallet (EVM, BTC, Solana, Tron). Quote-first; signing stays local.",
        how_to: "Use wallet.* RPC methods (balances, prepare_transfer, execute_prepared, tx_status, tx_receipt, lookup_tx) via the agent or core_rpc_relay, or via Settings > Crypto > Wallet Balances.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_CREDENTIALS,
    },
Capability {
        id: "workflows.web3_defi",
        name: "Web3 Swaps & Bridges",
        domain: "web3",
        category: CapabilityCategory::Workflows,
        description: "Quote and execute cross-chain swaps and bridges (deBridge) plus generic EVM dapp contract calls, built on the local wallet's signing. EVM/Solana(/BTC); signing stays local.",
        how_to: "Use web3_swap.* / web3_bridge.* / web3_dapp.* RPC methods (quote/execute, web3_swap.routes) via the agent or core_rpc_relay.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_CREDENTIALS,
    },
Capability {
        id: "workflows.x402_payments",
        name: "x402 Machine Payments",
        domain: "x402",
        category: CapabilityCategory::Workflows,
        description: "Automatic HTTP 402 payment handling for machine-payable APIs via the x402 protocol. When an API returns 402 Payment Required, the agent pays with USDC on Solana using the local wallet and retries. Budget enforcement with per-request, daily, and monthly caps.",
        how_to: "Use x402.* RPC methods (get_summary, list_payments, update_budget) to manage spending. Payments happen automatically when the http_request tool encounters a 402 with a PAYMENT-REQUIRED header.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_CREDENTIALS,
    },
Capability {
        id: "workflows.connect_crypto_exchange",
        name: "Connect Crypto Exchange",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Connect supported exchanges for trading and portfolio workflows.",
        how_to: "Connections",
        status: CapabilityStatus::ComingSoon,
        privacy: None,
    },
Capability {
        id: "automation.task_sources",
        name: "Task Sources",
        domain: "automation",
        category: CapabilityCategory::Automation,
        description: "Pull work items from GitHub, Notion, Linear, and ClickUp using per-source \
                      filters, then enrich them onto the agent's todo board and (for proactive \
                      sources) start an agent working on them.",
        how_to: "Settings > Task Sources",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
Capability {
        id: "automation.discover_workflows",
        name: "Suggested Workflows (Flow Scout)",
        domain: "flows",
        category: CapabilityCategory::Automation,
        description: "A read-only discovery agent (\"Flow Scout\") reads your memory, past \
                      conversations, connected apps, and existing flows to figure \
                      out which automations would actually help you, then proposes a handful of \
                      concrete, buildable workflow suggestions. Each card explains why it was \
                      suggested; \"Build this\" hands it to the workflow builder to author a real \
                      flow you review and save. Discovery never creates, enables, or runs a flow.",
        how_to: "Flows > Suggested for you > Discover",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
Capability {
        id: "automation.flow_memory_node",
        name: "Memory Node (Flows)",
        domain: "flows",
        category: CapabilityCategory::Automation,
        description: "A `memory` node inside a saved workflow graph, giving the flow direct, \
                      in-graph memory access with no agent turn involved. It can recall/search \
                      your durable, cross-flow memory \
                      (read-only — a flow can never write there) or from other flows' own \
                      memory (also read-only), and can remember/forget entries in its OWN \
                      private, flow-scoped memory namespace — never the user's personal memory, \
                      never another flow's. Every operation is gated by the flow's autonomy \
                      tier; a flow-scoped write can require human approval.",
        how_to: "Flows editor > add a `memory` node; set `config.operation` and `config.scope`.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
Capability {
        id: "automation.flow_dedup_node",
        name: "Dedup Node (Flows)",
        domain: "flows",
        category: CapabilityCategory::Automation,
        description: "A `dedup` node inside a saved workflow graph, giving the flow durable \
                      exactly-once processing per item with no agent turn or extra plumbing \
                      involved. It drops an item whose per-item key was already committed by a \
                      prior successful run, and otherwise passes it through. Committing happens \
                      automatically: keys the node passes through are marked done only once the \
                      whole run finishes successfully; a failed/cancelled/interrupted/unknown (or \
                      any other non-success) run leaves them unmarked so the same items retry next \
                      time. Only the resolved per-item key value is stored, locally, in the flow's \
                      own private, flow-scoped state — never the item's full content, and never \
                      the user's personal memory. The key is whatever the workflow author's \
                      `config.key` expression resolves to, so it can carry item-derived data if \
                      keyed off a sensitive field — author flows to key off an opaque, \
                      non-sensitive stable id (an issue number, message id, url) rather than \
                      personal data.",
        how_to: "Flows editor > add a `dedup` node right after the item source; set config.key \
                 to a stable per-item id expression, e.g. \"=item.id\".",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
Capability {
        id: "automation.view_cron_jobs",
        name: "View Cron Jobs",
        domain: "automation",
        category: CapabilityCategory::Automation,
        description: "Review scheduled jobs available to the runtime.",
        how_to: "Settings > Cron Jobs",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "automation.set_job_intervals",
        name: "Set Job Intervals",
        domain: "automation",
        category: CapabilityCategory::Automation,
        description: "Configure how often a scheduled job should run.",
        how_to: "Settings > Cron Jobs",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "automation.view_execution_history",
        name: "View Execution History",
        domain: "automation",
        category: CapabilityCategory::Automation,
        description: "Inspect past runs and results for scheduled jobs.",
        how_to: "Settings > Cron Jobs",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
Capability {
        id: "automation.morning_briefing",
        name: "Morning Briefing",
        domain: "automation",
        category: CapabilityCategory::Automation,
        description: "Daily proactive agent that reviews calendar, tasks, emails, and market context to deliver a morning summary.",
        how_to: "Automatic after onboarding (runs daily at 7 AM). Adjust schedule via Settings > Cron Jobs.",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
Capability {
        id: "automation.crypto_agent",
        name: "Crypto Wallet",
        domain: "automation",
        category: CapabilityCategory::Automation,
        description: "Wallet and market actions as a built-in skill (`web3`): quotes, \
                      swaps, bridges, contract calls and x402 payments. The assistant \
                      follows a read → quote → confirm → execute flow, refuses to \
                      fabricate chain ids or token addresses, and runs no swap, bridge \
                      or contract call without explicit user confirmation.",
        how_to: "Automatic — the assistant loads the web3 skill when a crypto wallet or market action is requested. Connect a wallet via Settings > Recovery Phrase first.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_CREDENTIALS,
    },
];
