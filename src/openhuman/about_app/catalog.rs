use std::collections::BTreeSet;
use std::sync::OnceLock;

use super::types::{
    Capability, CapabilityCategory, CapabilityPrivacy, CapabilityStatus, PrivacyDataKind,
};

const LOCAL_RAW: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: false,
    data_kind: PrivacyDataKind::Raw,
    destinations: &[],
});

const DERIVED_TO_BACKEND: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Derived,
    destinations: &["OpenHuman backend", "TinyHumans Neocortex"],
});

const LOCAL_CREDENTIALS: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: false,
    data_kind: PrivacyDataKind::Credentials,
    destinations: &[],
});

const DIAGNOSTICS_TO_BACKEND: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Diagnostics,
    destinations: &["OpenHuman backend"],
});

const MODEL_DOWNLOAD: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Metadata,
    destinations: &["Hugging Face"],
});

// Self-update flows talk to GitHub Releases directly, not the OpenHuman
// backend. The outbound payload is metadata only (release list query for
// `update.check`, asset download URL request for `update.apply`) so
// `data_kind: Metadata` is the right label — but the destination must
// reflect that this is a third-party host, otherwise the capability
// catalog under-reports where the user's request actually goes.
const GITHUB_RELEASES_METADATA: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Metadata,
    destinations: &["GitHub Releases"],
});

const CAPABILITIES: &[Capability] = &[
    Capability {
        id: "conversation.create",
        name: "Create Conversations",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Start a new conversation thread with the assistant.",
        how_to: "Conversations",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "conversation.send_text",
        name: "Send Text Messages",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Send typed messages to the assistant in a conversation.",
        how_to: "Conversations > Message composer",
        status: CapabilityStatus::Stable,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "conversation.prompt_injection_guard",
        name: "Prompt Injection Guard",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Detect and block prompt-injection attempts before agent/model execution.",
        how_to: "Conversations > Message composer",
        status: CapabilityStatus::Stable,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "conversation.send_voice",
        name: "Send Voice Messages",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Record or attach voice input and send it as a message.",
        how_to: "Conversations > Voice input",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "conversation.inline_autocomplete",
        name: "Inline Autocomplete",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Show predictive inline text suggestions while you type.",
        how_to: "Settings > Inline Autocomplete",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "conversation.copy_messages",
        name: "Copy Messages",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Copy individual assistant or user messages for reuse elsewhere.",
        how_to: "Conversations > Message actions",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "conversation.delete_conversations",
        name: "Delete Conversations",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Remove existing conversation threads from the app.",
        how_to: "Conversations > Thread actions",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "conversation.suggested_questions",
        name: "Suggested Questions",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Offer prompt suggestions to help continue a conversation.",
        how_to: "Home or Conversations > Suggested prompts",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "conversation.tool_execution_timeline",
        name: "Tool Execution Timeline",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Show the sequence of tool calls and actions used to answer a request.",
        how_to: "Conversations > Tool timeline",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "conversation.label_filter",
        name: "Thread Label Filters",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Filter the thread list by label (Work, Briefing, Notification) using the tab bar at the top of the thread list.",
        how_to: "Conversations > Label tabs",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "intelligence.analyze_actionable_items",
        name: "Analyze Actionable Items",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Extract and summarize actionable items from your activity and conversations.",
        how_to: "Intelligence",
        status: CapabilityStatus::Stable,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "intelligence.filter_actionable_items",
        name: "Filter Actionable Items",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Search and filter actionable items to focus on what matters now.",
        how_to: "Intelligence > Filters and search",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "intelligence.mark_actionable_item_complete",
        name: "Mark Items Complete",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Mark an actionable item as completed.",
        how_to: "Intelligence > Item actions",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "intelligence.dismiss_actionable_item",
        name: "Dismiss Items",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Dismiss irrelevant or already handled actionable items.",
        how_to: "Intelligence > Item actions",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "intelligence.snooze_actionable_item",
        name: "Snooze Items",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Temporarily hide an actionable item until later.",
        how_to: "Intelligence > Item actions",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "intelligence.undo_action",
        name: "Undo Item Actions",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Undo a recent complete, dismiss, or snooze action.",
        how_to: "Intelligence > Undo snackbar or item history",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "intelligence.memory_workspace",
        name: "Memory Workspace",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Inspect or debug the app's memory workspace and stored knowledge.",
        how_to: "Settings > Memory Debug",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "intelligence.tool_scoped_memory",
        name: "Tool-Scoped Memory Rules",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Store durable, tool-specific rules and corrections that survive context \
            compression. Critical-priority rules (e.g. 'never email Sarah') are pinned into the \
            system prompt at session start. Captured automatically from user edicts and repeated \
            tool failures; also writable programmatically via the memory.tool_rule_* RPC surface.",
        how_to: "Automatic — user edicts are captured after every turn. Manage via \
            memory.tool_rule_put / memory.tool_rule_list / memory.tool_rule_delete (RPC).",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "intelligence.memory_tree_retrieval",
        name: "Memory Tree Retrieval (chat)",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Ask questions about your ingested email/chat/document memory in chat. The orchestrator can resolve names to canonical ids, query summaries by source/topic/global window, drill into details, and cite raw chunks.",
        how_to: "Chat > ask the assistant about people, conversations, or windows",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "intelligence.orchestrator_worker_thread",
        name: "Worker Thread Delegation",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "When a delegated sub-task is long or complex, the orchestrator can route it into a fresh worker-labeled conversation thread instead of flooding the parent thread. The user opens the worker thread from the thread list (or via the reference card in the parent) to read the sub-agent's full transcript.",
        how_to: "Conversations > tap the worker reference card in the parent thread, or open the worker-labeled thread from the thread list",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "intelligence.slack_memory_ingest",
        name: "Slack Memory Ingestion",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Backfill the last 6 days of Slack history into the memory tree and keep it up to date by flushing each closed 6-hour UTC bucket. Driven by an authenticated Slack connection (OAuth via Composio).",
        how_to: "Settings > Messaging Channels > Slack",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "intelligence.notifications_dismiss",
        name: "Dismiss Notifications",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Dismiss low-value notifications from the intelligence inbox.",
        how_to: "Notifications > Item actions",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "intelligence.notifications_mark_acted",
        name: "Mark Notifications Acted",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Mark a notification as acted upon after taking follow-up action.",
        how_to: "Notifications > Item actions",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "intelligence.notifications_stats",
        name: "View Notification Stats",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "View aggregate unread, unscored, and provider/action notification stats.",
        how_to: "Notifications > Summary cards",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "skills.discover",
        name: "Discover Skills",
        domain: "skills",
        category: CapabilityCategory::Skills,
        description: "Browse available skills that can extend the app.",
        how_to: "Skills",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "skills.install",
        name: "Install Skills",
        domain: "skills",
        category: CapabilityCategory::Skills,
        description: "Install a skill into the local workspace.",
        how_to: "Skills > Install",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "skills.configure",
        name: "Configure Skills",
        domain: "skills",
        category: CapabilityCategory::Skills,
        description: "Open skill setup and update skill-specific configuration.",
        how_to: "Skills > Setup or Settings > Connections",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "skills.connection_status",
        name: "Monitor Skill Connection Status",
        domain: "skills",
        category: CapabilityCategory::Skills,
        description: "See whether a skill-backed integration is connected, offline, or needs setup.",
        how_to: "Skills or Settings > Connections",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "skills.sync_manual",
        name: "Manually Sync Skill Data",
        domain: "skills",
        category: CapabilityCategory::Skills,
        description: "Trigger a manual data sync for a skill integration.",
        how_to: "Skills > Skill card > Sync",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "skills.run_apify_actors",
        name: "Run Apify Actors",
        domain: "skills",
        category: CapabilityCategory::Skills,
        description: "Launch Apify scrapers and automation actors, then inspect run status and collected results.",
        how_to: "Conversations > Ask the assistant to run an Apify actor",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "skills.toggle_enabled",
        name: "Enable or Disable Skills",
        domain: "skills",
        category: CapabilityCategory::Skills,
        description: "Turn individual skills on or off without uninstalling them.",
        how_to: "Settings > Developer Options > Skills",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "skills.open_connections_hub",
        name: "Open Connections Hub",
        domain: "skills",
        category: CapabilityCategory::Skills,
        description: "Browse the dedicated connections hub for external skill-backed integrations.",
        how_to: "Settings > Connections",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "skills.connect_google",
        name: "Connect Google",
        domain: "skills",
        category: CapabilityCategory::Skills,
        description: "Connect Google services for email, contacts, and calendar workflows.",
        how_to: "Settings > Connections",
        status: CapabilityStatus::ComingSoon,
        privacy: LOCAL_CREDENTIALS,
    },
    Capability {
        id: "skills.connect_notion",
        name: "Connect Notion",
        domain: "skills",
        category: CapabilityCategory::Skills,
        description: "Connect Notion for workspace sync and productivity workflows.",
        how_to: "Settings > Connections",
        status: CapabilityStatus::ComingSoon,
        privacy: LOCAL_CREDENTIALS,
    },
    Capability {
        id: "skills.connect_web3_wallet",
        name: "Connect Web3 Wallet",
        domain: "skills",
        category: CapabilityCategory::Skills,
        description: "Set up local EVM, BTC, Solana, and Tron wallet identities from one recovery phrase.",
        how_to: "Settings > Recovery Phrase or Settings > Connections",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_CREDENTIALS,
    },
    Capability {
        id: "skills.wallet_execution",
        name: "Wallet Execution Tools",
        domain: "wallet",
        category: CapabilityCategory::Skills,
        description: "Read balances and prepare/confirm/execute transfers, swaps, and contract calls across the connected wallet (EVM, BTC, Solana, Tron). Quote-first; signing stays local.",
        how_to: "Use wallet.* RPC methods (balances, prepare_transfer, prepare_swap, prepare_contract_call, execute_prepared) via the agent or core_rpc_relay.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_CREDENTIALS,
    },
    Capability {
        id: "skills.connect_crypto_exchange",
        name: "Connect Crypto Exchange",
        domain: "skills",
        category: CapabilityCategory::Skills,
        description: "Connect supported exchanges for trading and portfolio workflows.",
        how_to: "Settings > Connections",
        status: CapabilityStatus::ComingSoon,
        privacy: None,
    },
    Capability {
        id: "local_ai.download_model",
        name: "Download Local Models",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Download and bootstrap local AI runtimes and model bundles.",
        how_to: "Settings > Local AI Model",
        status: CapabilityStatus::Beta,
        privacy: MODEL_DOWNLOAD,
    },
    Capability {
        id: "local_ai.manage_model_assets",
        name: "Manage Model Assets",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Inspect asset status and download specific chat, vision, embedding, STT, or TTS assets.",
        how_to: "Settings > Local AI Model > Advanced > Capability Assets",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "local_ai.embed_text",
        name: "Generate Text Embeddings",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Create local vector embeddings for text input.",
        how_to: "Settings > Local AI Model > Advanced > Test Embeddings",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "local_ai.speech_to_text",
        name: "Speech Recognition",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Transcribe audio into text using local speech recognition.",
        how_to: "Settings > Local AI Model > Advanced > Test Voice Input",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "local_ai.text_to_speech",
        name: "Text to Speech",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Synthesize speech from text using local voice models.",
        how_to: "Settings > Local AI Model > Advanced > Test Voice Output",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "local_ai.vision_processing",
        name: "Vision Processing",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Run vision prompts against images using a local multimodal model.",
        how_to: "Settings > Local AI Model > Advanced > Test Vision Prompt",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "local_ai.direct_prompting",
        name: "Direct Model Prompting",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Send a direct prompt to the local model without using the cloud API.",
        how_to: "Settings > Local AI Model > Advanced > Test Custom Prompt",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "team.create",
        name: "Create Teams",
        domain: "team",
        category: CapabilityCategory::Team,
        description: "Create a team and start collaborating with shared billing and members.",
        how_to: "Settings > Team",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "team.join_via_invite_code",
        name: "Join Teams via Invite Code",
        domain: "team",
        category: CapabilityCategory::Team,
        description: "Join an existing team using an invite code.",
        how_to: "Invites > Redeem an Invite Code",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "team.switch_active_team",
        name: "Switch Active Team",
        domain: "team",
        category: CapabilityCategory::Team,
        description: "Switch which team is currently active in the app.",
        how_to: "Settings > Team",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "team.leave",
        name: "Leave Teams",
        domain: "team",
        category: CapabilityCategory::Team,
        description: "Leave a team that you no longer want to participate in.",
        how_to: "Settings > Team",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "team.manage_members",
        name: "Manage Team Members",
        domain: "team",
        category: CapabilityCategory::Team,
        description: "Review members and change team roles when you have permission.",
        how_to: "Settings > Team > Manage team > Members",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "team.generate_invite_codes",
        name: "Generate Invite Codes",
        domain: "team",
        category: CapabilityCategory::Team,
        description: "Create invite codes to bring new members into a team.",
        how_to: "Settings > Team > Manage team > Invites",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "team.track_invite_usage",
        name: "Track Invite Usage",
        domain: "team",
        category: CapabilityCategory::Team,
        description: "View invite usage counts, limits, and revoke team invites.",
        how_to: "Settings > Team > Manage team > Invites",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "auth.login_oauth",
        name: "Login via OAuth",
        domain: "auth",
        category: CapabilityCategory::Auth,
        description: "Sign in with the app's supported provider-based authentication flow.",
        how_to: "Welcome",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "auth.onboarding_setup",
        name: "Onboarding Setup",
        domain: "auth",
        category: CapabilityCategory::Auth,
        description: "Walk through onboarding to configure initial permissions and preferences.",
        how_to: "Onboarding",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "auth.configure_tool_access",
        name: "Configure Tool Access",
        domain: "auth",
        category: CapabilityCategory::Auth,
        description: "Choose which built-in tools OpenHuman can use on your behalf during setup.",
        how_to: "Onboarding > Enable Tools",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "auth.backup_recovery_phrase",
        name: "Back Up Recovery Phrase",
        domain: "auth",
        category: CapabilityCategory::Auth,
        description: "Generate and save a recovery phrase used to secure and restore encrypted app data.",
        how_to: "Onboarding > Recovery Phrase",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "auth.import_recovery_phrase",
        name: "Import Recovery Phrase",
        domain: "auth",
        category: CapabilityCategory::Auth,
        description: "Import an existing recovery phrase to restore encrypted app data.",
        how_to: "Onboarding > Recovery Phrase > I already have a recovery phrase",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "auth.logout",
        name: "Logout",
        domain: "auth",
        category: CapabilityCategory::Auth,
        description: "Sign out of the current session.",
        how_to: "Settings > Log out",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "screen_intelligence.toggle_monitoring",
        name: "Enable or Disable Screen Monitoring",
        domain: "screen_intelligence",
        category: CapabilityCategory::ScreenIntelligence,
        description: "Turn desktop screen intelligence capture on or off.",
        how_to: "Settings > Screen Intelligence",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "screen_intelligence.manage_accessibility_permissions",
        name: "Manage Accessibility Permissions",
        domain: "screen_intelligence",
        category: CapabilityCategory::ScreenIntelligence,
        description: "Review and grant the accessibility permissions required for desktop assistance.",
        how_to: "Onboarding > Screen permissions or Settings > Accessibility Automation",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "screen_intelligence.review_vision_data",
        name: "Review Vision Data",
        domain: "screen_intelligence",
        category: CapabilityCategory::ScreenIntelligence,
        description: "Inspect the captured screen intelligence and related vision summaries.",
        how_to: "Settings > Screen Intelligence",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "screen_intelligence.configure_capture_fps",
        name: "Configure Capture FPS",
        domain: "screen_intelligence",
        category: CapabilityCategory::ScreenIntelligence,
        description: "Tune the screen capture frame rate used by screen intelligence.",
        how_to: "Settings > Screen Intelligence",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "screen_intelligence.app_whitelist",
        name: "Whitelist Apps for Capture",
        domain: "screen_intelligence",
        category: CapabilityCategory::ScreenIntelligence,
        description: "Allow screen intelligence only for selected applications.",
        how_to: "Settings > Screen Intelligence",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "screen_intelligence.app_blacklist",
        name: "Blacklist Apps from Capture",
        domain: "screen_intelligence",
        category: CapabilityCategory::ScreenIntelligence,
        description: "Exclude selected applications from screen intelligence capture.",
        how_to: "Settings > Screen Intelligence",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "channels.connect_platform",
        name: "Connect Messaging Platforms",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description: "Connect supported messaging platforms such as Telegram, Discord, or Slack.",
        how_to: "Settings > Messaging Channels",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "channels.disconnect_platform",
        name: "Disconnect Messaging Platforms",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description: "Disconnect a previously configured messaging platform.",
        how_to: "Settings > Messaging Channels",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "channels.test_credentials",
        name: "Test Channel Credentials",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description: "Validate platform credentials or connection state before using a channel.",
        how_to: "Settings > Messaging Channels",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "channels.set_default_channel",
        name: "Set Default Messaging Channel",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description: "Choose which messaging channel should be used by default.",
        how_to: "Settings > Messaging Channels",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "channels.whatsapp_read_messages",
        name: "Read WhatsApp Messages",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description: "Read and search WhatsApp Web conversations and messages after connecting WhatsApp in OpenHuman. Data is stored locally only and never transmitted.",
        how_to: "Connect WhatsApp Web via Channels, then ask the agent to read or summarise your messages.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "settings.configure_ai",
        name: "Configure AI",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Adjust AI-related settings and agent behavior preferences.",
        how_to: "Settings > Developer Options > AI Configuration",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "settings.manage_privacy_analytics",
        name: "Manage Privacy and Analytics",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Control privacy, analytics, and related data handling preferences.",
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
        description: "Open developer-focused panels for diagnostics, skills, AI config, and memory tools.",
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
        description: "Sign out and permanently clear local app data, including skills data.",
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
    // ── Proactive agents ─────────────────────────────────────────────────────
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
        id: "automation.welcome_agent",
        name: "Welcome Message",
        domain: "automation",
        category: CapabilityCategory::Automation,
        description: "Conversational onboarding agent that learns about the user's intent and daily tools before guiding them through personalized setup.",
        how_to: "Automatic — triggered once after onboarding.",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    // ── Update ──────────────────────────────────────────────────────────────
    // ── Meet ────────────────────────────────────────────────────────────────
    Capability {
        id: "meet.join_call",
        name: "Join Google Meet Calls",
        domain: "meet",
        category: CapabilityCategory::Channels,
        description: "Join a Google Meet call as an anonymous guest in a dedicated CEF webview \
                      window with an isolated profile. The agent automatically dismisses the \
                      device-check, types its display name, and clicks Ask-to-join via CDP; the \
                      host admits the agent from the Meet waiting room.",
        how_to: "Intelligence > Calls",
        status: CapabilityStatus::Beta,
        privacy: Some(CapabilityPrivacy {
            leaves_device: true,
            data_kind: PrivacyDataKind::Metadata,
            destinations: &["Google Meet"],
        }),
    },
    Capability {
        id: "meet_agent.live_loop",
        name: "Live Meet Agent — Listen + Speak",
        domain: "meet_agent",
        category: CapabilityCategory::Automation,
        description: "While the agent is in a Google Meet call, it listens to the other \
                      participants by tapping the embedded webview's audio output, runs \
                      VAD-segmented speech-to-text, decides whether to respond, and speaks \
                      back through a virtual microphone the embedded Chromium reads as if \
                      it were a real input device. No system audio permission required — \
                      capture and playback both stay inside the CEF process.",
        how_to: "Automatic once a Meet call is open via Intelligence > Calls.",
        status: CapabilityStatus::Beta,
        privacy: Some(CapabilityPrivacy {
            leaves_device: true,
            data_kind: PrivacyDataKind::Derived,
            destinations: &["Google Meet", "ElevenLabs (STT/TTS via hosted backend)"],
        }),
    },
    // ── Update ──────────────────────────────────────────────────────────────
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
];

static VALIDATED: OnceLock<()> = OnceLock::new();

pub fn all_capabilities() -> &'static [Capability] {
    ensure_validated();
    CAPABILITIES
}

pub fn capabilities_by_category(category: CapabilityCategory) -> Vec<Capability> {
    ensure_validated();
    CAPABILITIES
        .iter()
        .filter(|capability| capability.category == category)
        .copied()
        .collect()
}

pub fn lookup(id: &str) -> Option<Capability> {
    ensure_validated();
    let normalized = id.trim();
    CAPABILITIES
        .iter()
        .find(|capability| capability.id == normalized)
        .copied()
}

pub fn search(query: &str) -> Vec<Capability> {
    ensure_validated();
    let normalized = query.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return CAPABILITIES.to_vec();
    }

    CAPABILITIES
        .iter()
        .filter(|capability| searchable_text(capability).contains(&normalized))
        .copied()
        .collect()
}

fn searchable_text(capability: &Capability) -> String {
    format!(
        "{} {} {} {} {} {} {}",
        capability.id,
        capability.name,
        capability.domain,
        capability.category.as_str(),
        capability.description,
        capability.how_to,
        capability.status.as_str()
    )
    .to_ascii_lowercase()
}

fn ensure_validated() {
    VALIDATED.get_or_init(|| {
        let mut ids = BTreeSet::new();
        for capability in CAPABILITIES {
            assert!(
                !capability.id.trim().is_empty(),
                "about_app capability id must not be empty"
            );
            assert!(
                ids.insert(capability.id),
                "duplicate about_app capability id '{}'",
                capability.id
            );
        }

        tracing::debug!(
            count = CAPABILITIES.len(),
            "[about_app] validated capability catalog"
        );
    });
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
