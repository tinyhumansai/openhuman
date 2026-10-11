//! LocalAI, Settings, and Mobile capability entries.

use super::*;

pub(super) const CAPABILITIES: &[Capability] = &[
Capability {
        id: "local_ai.configure_provider",
        name: "Configure Local Provider",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Point OpenHuman at a local runtime you run yourself: Ollama, LM Studio, MLX, oMLX, or a generic OpenAI-compatible server. OpenHuman does not install the runtime or download models; pull them in your runtime (e.g. `ollama pull <model>`). Local chat inference does not require an OpenHuman account.",
        how_to: "Connections → API keys → LLM, or use provider strings: ollama:<model>, lmstudio:<model>, mlx:<model>, omlx:<model>, local-openai:<model>",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
Capability {
        id: "local_ai.model_context_check",
        name: "Model Context Requirement Check",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Diagnostics report each installed Ollama model's native context window and reject any model below the minimum the memory layer requires (so short-context models can't silently truncate and corrupt recall).",
        how_to: "Reported by the core `inference_diagnostics` method; no settings control exposes it since the local-model panel was removed (0ec68613af).",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
Capability {
        id: "local_ai.embed_text",
        name: "Generate Text Embeddings",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Create local vector embeddings for text input.",
        how_to: "Connections → API keys → Embeddings, or the core `embeddings_embed` method.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
Capability {
        id: "local_ai.text_to_speech",
        name: "Text to Speech (Local)",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description:
            "Synthesize speech locally with Piper via the voice TTS factory. PIPER_BIN points \
             at the binary; the voice .onnx ships with the installer. Returns a synthetic \
             viseme timeline (full forced-alignment lives behind the cloud provider for now).",
        how_to: "Settings > Voice > TTS Provider = Piper",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
Capability {
        id: "local_ai.vision_processing",
        name: "Vision Processing",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Run vision prompts against images using a local multimodal model.",
        how_to: "Served by the core `inference_vision_prompt` method; no settings control exposes it since the local-model panel was removed (0ec68613af).",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
Capability {
        id: "local_ai.direct_prompting",
        name: "Direct Model Prompting",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Send a direct prompt to the local model without using the cloud API.",
        how_to: "Served by the core `inference_prompt` method; no settings control exposes it since the local-model panel was removed (0ec68613af).",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
Capability {
        id: "settings.configure_ai",
        name: "Configure AI",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Configure managed, local, custom, and built-in BYOK LLM providers, including SumoPod and other OpenAI-compatible gateways. Custom providers can trust an uploaded PEM CA certificate for model listing and inference. Set per-workload routing preferences here too.",
        how_to: "Connections → API keys → LLM",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "settings.desktop_control",
        name: "Desktop Control",
        domain: "desktop",
        category: CapabilityCategory::Settings,
        description: "Let agents inspect and control native desktop apps through a local, permission-aware connection. Desktop actions are discovered on demand.",
        how_to: "Connections → Computer → Desktop; enable access and grant operating system permissions when requested",
        status: CapabilityStatus::Beta,
        privacy: DESKTOP_TO_JEV,
    },
Capability {
        id: "settings.persona_pack",
        name: "Choose Assistant Mascot",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Browse the published mascot library, choose an assistant face, and configure its visual appearance and voice.",
        how_to: "Settings > Personality > Face",
        status: CapabilityStatus::Stable,
        privacy: GITHUB_MASCOT_MANIFEST,
    },
Capability {
        id: "settings.core_connection",
        name: "Run the Core Somewhere Else",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Choose where the OpenHuman core runs: inside this app, at a URL \
            you point it at, in a Docker container, on another machine over SSH, or in a \
            container on another machine. OpenHuman starts the core, connects to it, and \
            shuts it down when you switch away. Where it runs and what contains it are \
            separate choices, so SSH and Docker combine without being a third option.",
        how_to: "Settings > Core connection",
        status: CapabilityStatus::Beta,
        privacy: Some(CapabilityPrivacy {
            leaves_device: true,
            data_kind: PrivacyDataKind::Raw,
            // `Raw`, because everything the core holds - the conversations
            // themselves, memory, stored credentials - lives wherever the core
            // runs. Choosing a machine over SSH is choosing to put all of it
            // there. That is the disclosure that matters here and the one a
            // hostname field makes easy to overlook, so it is stated at the
            // strongest kind rather than softened to `Derived`.
            destinations: &["The machine you configure, when the core runs off this device"],
        }),
    },
Capability {
        id: "settings.manage_privacy_analytics",
        name: "Manage Privacy and Analytics",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Control privacy, analytics, and related data handling preferences. \
            When enabled, anonymous crash reports are sent to Sentry and anonymous usage \
            analytics (page views, feature engagement) are sent to Google Analytics. \
            No personal data, messages, or credentials are ever included.",
        how_to: "Settings > Privacy (direct route)",
        status: CapabilityStatus::Stable,
        privacy: DIAGNOSTICS_TO_BACKEND,
    },
Capability {
        id: "settings.view_billing",
        name: "View Billing",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Open subscription, included usage, and pay-as-you-go billing views for your active team.",
        how_to: "Settings > Billing & Usage",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "settings.manage_subscription_plan",
        name: "Manage Subscription Plan",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Upgrade plans or open the billing portal to manage subscription-backed usage tiers.",
        how_to: "Settings > Billing & Usage",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "settings.manage_credits",
        name: "Manage Credits",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "View pay-as-you-go credit balances, top up overage credits, and configure auto-recharge.",
        how_to: "Settings > Billing & Usage",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "settings.add_payment_methods",
        name: "Add Payment Methods",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Add or manage saved payment methods for billing and auto-recharge.",
        how_to: "Settings > Billing & Usage > Payment Methods",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "settings.developer_options",
        name: "Developer Options",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Open developer-focused panels for diagnostics, workflows, AI config, and memory tools.",
        how_to: "Settings > Developer Options",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
Capability {
        id: "settings.debug_webhooks",
        name: "Debug Webhooks",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description:
            "Inspect Composio trigger history and find the daily JSONL archive files stored by the app.",
        how_to: "Settings > Developer Options > Webhooks",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
Capability {
        id: "settings.manage_service",
        name: "Manage Desktop Service",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Install, start, stop, restart, uninstall, or inspect the optional desktop background service.",
        how_to: "Settings > Developer Options > Tauri Commands",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "settings.clear_app_data",
        name: "Log Out and Clear App Data",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Sign out and permanently clear local app data, including workflow data.",
        how_to: "Settings > Log Out & Clear App Data",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "settings.delete_all_data",
        name: "Delete All Data",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Delete all local data and reset the app from the destructive settings section.",
        how_to: "Settings > Delete All Data",
        status: CapabilityStatus::ComingSoon,
        privacy: None,
    },
Capability {
        id: "mobile.device_pairing",
        name: "Device Pairing",
        domain: "devices",
        category: CapabilityCategory::Mobile,
        description: "Pair iOS phones with the desktop core via QR code. The desktop generates a \
                      short-lived pairing token; the iOS app scans the QR, completes an X25519 \
                      key agreement, and stores the session for reconnects.",
        how_to: "Settings > Devices > Pair iPhone",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
Capability {
        id: "mobile.ios_client",
        name: "iOS Client",
        domain: "devices",
        category: CapabilityCategory::Mobile,
        description: "iOS app for chatting with your assistant on the go. Connects to the desktop \
                      core via LAN HTTP, an E2E-encrypted socket.io tunnel, or a cloud HTTP \
                      fallback — no Rust core ships on the device.",
        how_to: "Pair via Settings > Devices, then open the OpenHuman iOS app.",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
Capability {
        id: "mobile.push_to_talk",
        name: "Push-to-Talk",
        domain: "devices",
        category: CapabilityCategory::Mobile,
        description: "Hold-to-talk voice input on iOS. Activates AVAudioEngine and \
                      SFSpeechRecognizer on the device; partial transcripts appear while \
                      speaking and the final transcript is sent as a chat message.",
        how_to: "Hold the microphone button on the iOS mascot screen.",
        status: CapabilityStatus::Beta,
        privacy: Some(CapabilityPrivacy {
            leaves_device: false,
            data_kind: PrivacyDataKind::Raw,
            destinations: &[],
        }),
    },
Capability {
        id: "update.check",
        name: "Check for Core Updates",
        domain: "update",
        category: CapabilityCategory::Settings,
        description: "Query GitHub Releases to see if a newer core binary is available. \
                      Available to the orchestrator agent as the `update_check` tool so the \
                      user can ask 'am I up to date?' in chat.",
        how_to: "Settings > Developer Options > Check for Updates, or ask the orchestrator in chat.",
        status: CapabilityStatus::Beta,
        privacy: GITHUB_RELEASES_METADATA,
    },
Capability {
        id: "update.apply",
        name: "Apply Core Update",
        domain: "update",
        category: CapabilityCategory::Settings,
        description: "Download and stage a newer core binary. Desktop builds can self-restart; \
                      headless deployments can hand restart off to a supervisor. Exposed to \
                      the orchestrator agent as the `update_apply` tool, gated behind explicit \
                      user consent (the agent must confirm via `ask_user_clarification` before \
                      invoking) and the `config.update.rpc_mutations_enabled` policy switch.",
        how_to: "Settings > Developer Options > Apply Update, or confirm an in-chat update prompt from the orchestrator.",
        status: CapabilityStatus::Beta,
        privacy: GITHUB_RELEASES_METADATA,
    },
Capability {
        id: "filesystem.access_mode",
        name: "Agent OS Access Mode",
        domain: "security",
        category: CapabilityCategory::Settings,
        description: "Choose how much filesystem and shell access the agent has: Read-Only, \
                      Workspace, Trusted Roots (grant specific folders outside the workspace), \
                      or Full Access. Credential stores stay blocked in every mode.",
        how_to: "Settings → Agent OS access",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "agent.action_timeout",
        name: "Action Timeout",
        domain: "agent",
        category: CapabilityCategory::Settings,
        description: "Set how long a single tool or action may run before it is cancelled \
                      (1–3600 seconds, default 120). Increase it when a large local model is \
                      interrupted before finishing its response. Applies to the next tool call \
                      without a restart; the OPENHUMAN_TOOL_TIMEOUT_SECS env var still overrides it.",
        how_to: "Settings → Agent OS access → Action timeout",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "security.always_allow_tool",
        name: "Always Allow a Tool",
        domain: "security",
        category: CapabilityCategory::Settings,
        description: "On an approval prompt, choose \"Always allow\" to stop being asked for that \
                      tool. The choice is saved to your allow-list and persists across restarts; \
                      remove it any time under Settings → Agent OS access to be prompted again. \
                      Policy still blocks forbidden paths and high-risk commands regardless.",
        how_to: "Click \"Always allow\" on an approval prompt; manage the list in Settings → Agent OS access.",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "security.approval_history",
        name: "Approval History",
        domain: "security",
        category: CapabilityCategory::Settings,
        description: "Review a read-only audit trail of past tool-approval decisions \
                      (Approve once / Always allow / Deny), newest first. Summaries are \
                      scrubbed of chat content and arguments are shown as redacted shape only.",
        how_to: "Settings → Agent OS access → View approval history",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "tool.detect_tools",
        name: "Detect Installed Tools",
        domain: "tools",
        category: CapabilityCategory::Settings,
        description: "Probe the host PATH to report which developer tools and language \
                      runtimes are installed (node, python, cargo, docker, git, …).",
        how_to: "Used by the agent automatically; gated by the tool toggle list.",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "tool.install_tool",
        name: "Install OS Packages",
        domain: "tools",
        category: CapabilityCategory::Settings,
        description: "Install OS or language packages (apt/dnf/brew/winget/pipx/npm/cargo). \
                      High impact: only available when Full access / tool installation is enabled.",
        how_to: "Enable in Settings → Agent OS access (Full access mode).",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
Capability {
        id: "security.action_sandbox",
        name: "Action Sandbox",
        domain: "security",
        category: CapabilityCategory::Settings,
        description: "Dedicated action directory for agent tools (shell, file, git), separate \
                      from internal application state. Agent tools default their working directory \
                      and path resolution to the action sandbox, preventing accidental modification \
                      of memory databases, session transcripts, tokens, and other internal state.",
        how_to: "Settings → Agent OS access",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
Capability {
        id: "security.sandbox_backends",
        name: "Sandbox Execution Backends",
        domain: "security",
        category: CapabilityCategory::Settings,
        description: "Route agent tool execution (shell, filesystem, process) through sandbox \
                      backends — Docker containers or OS-level jails (Landlock/Seatbelt) — for \
                      reduced blast radius on remote, channel, cron, or background sessions. \
                      Configurable per agent/session/channel with safe defaults for non-main sessions.",
        how_to: "Set sandbox_mode = \"sandboxed\" in agent.toml, or configure runtime.kind = \
                 \"docker\" in the TOML config. Use openhuman.sandbox_status / \
                 openhuman.sandbox_resolve_policy RPC to inspect.",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
];
