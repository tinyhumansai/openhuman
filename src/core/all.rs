//! Registry and dispatch logic for all OpenHuman controllers.
//!
//! This module serves as the central hub for registering domain-specific
//! controllers (e.g., memory, skills, config) and providing a unified
//! interface for both the CLI and RPC layers to invoke them.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use serde_json::{Map, Value};

use crate::core::ControllerSchema;

/// A pinned, boxed future returned by a controller handler.
pub type ControllerFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'static>>;

/// A function pointer type for controller handlers.
///
/// Handlers take a map of parameters and return a [`ControllerFuture`].
pub type ControllerHandler = fn(Map<String, Value>) -> ControllerFuture;

/// A function pointer type for domain-specific CLI handlers.
pub type CliHandler = fn(&[String]) -> anyhow::Result<()>;

/// A registered standalone CLI adapter for a domain.
#[derive(Clone)]
pub struct RegisteredCliAdapter {
    pub namespace: &'static str,
    pub handler: CliHandler,
}

/// A registered controller combining its schema and handler function.
#[derive(Clone)]
pub struct RegisteredController {
    /// The schema defining the controller's identity and parameters.
    pub schema: ControllerSchema,
    /// The actual function that executes the controller's logic.
    pub handler: ControllerHandler,
}

impl RegisteredController {
    /// Returns the canonical RPC method name for this controller (e.g., `openhuman.memory_doc_put`).
    pub fn rpc_method_name(&self) -> String {
        rpc_method_name(&self.schema)
    }
}

/// Coarse-grained domain *family* a controller belongs to, used to gate its live
/// surface by the ambient [`crate::core::runtime::DomainSet`] (#4796).
///
/// Every registered controller is tagged with exactly one group at its single
/// registration site ([`build_registered_controllers`] /
/// [`build_internal_only_controllers`]); the live surface (schema dump,
/// dispatch, agent tools, stores, subscribers) filters by whether the active
/// [`crate::core::runtime::context::CoreContext`]'s `DomainSet` allows that
/// group. `full()` allows every group ⇒ registration is byte-identical to
/// pre-#4796. When no context is active (unit tests before boot) filtering is
/// disabled (treated as full).
///
/// The harness families (`Agent`/`Memory`/`Threads`/`Config`/`Security`) are on
/// under [`crate::core::runtime::DomainSet::harness`]; the gate families
/// (`Flows`/`Skills`/`Mcp`/`Meet`/`Channels`/`Web3`/`Voice`/`Media`) are the
/// per-feature axes the child issues (#4797–#4804) additionally narrow at
/// compile time. `Platform` is the catch-all for everything not in a named
/// family — always on in `full()`, off in `harness()`/`none()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DomainGroup {
    // Harness families — on under `DomainSet::harness()`.
    Agent,
    Memory,
    Threads,
    Config,
    Security,
    // Gate families — off under `harness()`; per-gate Cargo features (#4797–#4804)
    // narrow these further at compile time.
    Flows,
    Skills,
    Mcp,
    Meet,
    Channels,
    Web3,
    Voice,
    Media,
    // Everything not in a named family — always on in `full()`, off otherwise.
    Platform,
}

/// A [`RegisteredController`] tagged with the [`DomainGroup`] it belongs to.
///
/// The registry stores these so the live surface can be filtered by the ambient
/// [`crate::core::runtime::DomainSet`] WITHOUT touching the ~109 domain modules'
/// bare `RegisteredController { schema, handler }` literals — the group tag is
/// attached once, here, at the single registration site.
#[derive(Clone)]
struct GroupedController {
    group: DomainGroup,
    controller: RegisteredController,
}

/// Append `items` to `dst`, tagging each with `group`. This is the single seam
/// that attaches a [`DomainGroup`] to every domain's controllers without the
/// domain modules knowing about groups.
fn push(dst: &mut Vec<GroupedController>, group: DomainGroup, items: Vec<RegisteredController>) {
    dst.extend(
        items
            .into_iter()
            .map(|controller| GroupedController { group, controller }),
    );
}

/// The [`DomainSet`](crate::core::runtime::DomainSet) of the ambient dispatch
/// context, if any. `None` before any [`crate::core::runtime::context::CoreContext`]
/// is built (some unit tests) ⇒ callers treat that as "no filtering" (full).
fn active_domain_set() -> Option<crate::core::runtime::DomainSet> {
    crate::core::runtime::context::CoreContext::current().map(|c| c.domains())
}

/// Whether the given [`DomainGroup`] is enabled under the ambient
/// [`DomainSet`](crate::core::runtime::DomainSet). No active context ⇒ `true`
/// (full, no filter) so pre-boot unit tests and non-context callers see every
/// domain, exactly as before #4796.
fn group_allowed(group: DomainGroup) -> bool {
    active_domain_set().is_none_or(|s| s.allows(group))
}

/// The global static registry of all controllers, initialized once on first access.
static REGISTRY: OnceLock<Vec<GroupedController>> = OnceLock::new();

/// Internal-only controllers: registered for RPC dispatch but NOT in the agent-facing
/// schema catalog.  These handlers are callable by trusted callers (e.g. the Tauri scanner)
/// but should not be advertised to agents via tool listings or schema discovery.
static INTERNAL_REGISTRY: OnceLock<Vec<GroupedController>> = OnceLock::new();

/// The global static registry of standalone CLI adapters.
static CLI_ADAPTERS: OnceLock<Vec<RegisteredCliAdapter>> = OnceLock::new();

/// Returns a reference to the global controller registry.
///
/// This function initializes the registry if it hasn't been already,
/// performing validation to ensure no duplicates or missing handlers exist.
fn registry() -> &'static [GroupedController] {
    REGISTRY
        .get_or_init(|| {
            let registered = build_registered_controllers();
            // Drift guard runs once on the FULL set — validation is independent
            // of any ambient DomainSet filter (which only affects the live
            // surface, never registry integrity).
            validate_registry(&registered).unwrap_or_else(|err| {
                panic!("invalid controller registry: {err}");
            });
            registered
        })
        .as_slice()
}

/// Returns a reference to the internal-only controller registry.
///
/// These controllers are callable over RPC but are NOT included in agent tool listings
/// or schema discovery endpoints.
fn internal_registry() -> &'static [GroupedController] {
    INTERNAL_REGISTRY
        .get_or_init(build_internal_only_controllers)
        .as_slice()
}

/// Returns a reference to the global CLI adapter registry.
fn cli_adapters() -> &'static [RegisteredCliAdapter] {
    CLI_ADAPTERS.get_or_init(|| {
        vec![RegisteredCliAdapter {
            namespace: "voice",
            handler: crate::openhuman::voice::cli::run_standalone_subcommand,
        }]
    })
}

/// Aggregates all controller implementations from across the codebase.
///
/// This function is responsible for collecting every domain-specific controller
/// registered in the system. It is used during the initialization of the
/// global [`REGISTRY`].
///
/// When adding a new domain/namespace, its `all_*_registered_controllers()`
/// function must be called here to make it available via RPC and CLI.
fn build_registered_controllers() -> Vec<GroupedController> {
    let mut controllers = Vec::new();
    // Application information and capabilities
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::about_app::all_about_app_registered_controllers(),
    );
    // AgentBox marketplace adapter status
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::agentbox::all_agentbox_registered_controllers(),
    );
    // Core application shell state
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::app_state::all_app_state_registered_controllers(),
    );
    // Audio generation + podcast-style email delivery
    push(
        &mut controllers,
        DomainGroup::Voice,
        crate::openhuman::audio_toolkit::all_audio_toolkit_registered_controllers(),
    );
    // Composio integration controllers
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::composio::all_composio_registered_controllers(),
    );
    // Recall.ai Calendar V1 (backend-proxied) controllers
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::recall_calendar::all_recall_calendar_registered_controllers(),
    );
    // Scheduled job management
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::cron::all_cron_registered_controllers(),
    );
    // Saved automation workflows (tinyflows graphs): create/get/list/update/delete/run
    push(
        &mut controllers,
        DomainGroup::Flows,
        crate::openhuman::flows::all_flows_registered_controllers(),
    );
    // Proactive task ingestion from external tools (github/notion/linear/clickup)
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::task_sources::all_task_sources_registered_controllers(),
    );
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::dashboard::all_dashboard_registered_controllers(),
    );
    // MCP client subsystem: Smithery registry browser, local server install/connect, tool dispatch
    push(
        &mut controllers,
        DomainGroup::Mcp,
        crate::openhuman::mcp_registry::all_mcp_registry_registered_controllers(),
    );
    // Webview APIs bridge — proxies connector calls (Gmail, …) through
    // a WebSocket to the Tauri shell so curl reaches the live webview.
    push(
        &mut controllers,
        DomainGroup::Channels,
        crate::openhuman::webview_apis::all_webview_apis_registered_controllers(),
    );
    // Agent definition and prompt inspection
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::openhuman::agent::all_agent_registered_controllers(),
    );
    // Read-only agent run replay + status over the durable journal/status seams
    // (agent_run_events / agent_run_status / agent_runs_active).
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::openhuman::tinyagents::replay::all_agent_replay_registered_controllers(),
    );
    // Persistent agent profiles (flavours): name, soul, memory sources, skills, MCP, connectors.
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::openhuman::profiles::all_profiles_registered_controllers(),
    );
    // User-facing agent registry: defaults, enablement, custom agents, tool policy.
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::openhuman::agent_registry::all_agent_registry_registered_controllers(),
    );
    // Local procedural operating experience for agent self-learning
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::openhuman::agent_experience::all_agent_experience_registered_controllers(),
    );
    // System and process health monitoring
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::health::all_health_registered_controllers(),
    );
    // One-time first-run initialization (Python/spaCy/Node provisioning)
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::harness_init::all_harness_init_registered_controllers(),
    );
    // Diagnostic tools
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::doctor::all_doctor_registered_controllers(),
    );
    // Secret storage and encryption
    push(
        &mut controllers,
        DomainGroup::Security,
        crate::openhuman::encryption::all_encryption_registered_controllers(),
    );
    // Keyring consent — user approval before local secret storage fallback
    push(
        &mut controllers,
        DomainGroup::Security,
        crate::openhuman::keyring_consent::all_keyring_consent_registered_controllers(),
    );
    // Security policy metadata
    push(
        &mut controllers,
        DomainGroup::Security,
        crate::openhuman::security::all_security_registered_controllers(),
    );
    // Interactive approval workflow (#1339 — gate external-effect tool calls)
    push(
        &mut controllers,
        DomainGroup::Security,
        crate::openhuman::approval::all_approval_registered_controllers(),
    );
    // Interactive plan-review gate — parks a live turn on a thread-scoped plan
    push(
        &mut controllers,
        DomainGroup::Security,
        crate::openhuman::plan_review::all_plan_review_registered_controllers(),
    );
    // Agent-generated artifact storage, retrieval, and lifecycle management
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::artifacts::all_artifacts_registered_controllers(),
    );
    // Background heartbeat loop controls
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::heartbeat::all_heartbeat_registered_controllers(),
    );
    // Ad-hoc static directory HTTP hosting for local file sharing / previews
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::http_host::all_http_host_registered_controllers(),
    );
    // Token usage and billing cost tracking
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::cost::all_cost_registered_controllers(),
    );
    // x402 machine-payable API payment protocol
    push(
        &mut controllers,
        DomainGroup::Web3,
        crate::openhuman::x402::all_x402_registered_controllers(),
    );
    // Inline autocomplete settings
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::autocomplete::all_autocomplete_registered_controllers(),
    );
    // External messaging channels (Web, Telegram, etc.)
    push(
        &mut controllers,
        DomainGroup::Channels,
        crate::openhuman::channels::providers::web::all_web_channel_registered_controllers(),
    );
    push(
        &mut controllers,
        DomainGroup::Channels,
        crate::openhuman::channels::controllers::all_channels_registered_controllers(),
    );
    // Persistent configuration management
    push(
        &mut controllers,
        DomainGroup::Config,
        crate::openhuman::config::all_config_registered_controllers(),
    );
    // Local sidecar reachability + backend Socket.IO state diagnostics (#1527)
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::connectivity::all_connectivity_registered_controllers(),
    );
    // User credentials and session management
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::credentials::all_credentials_registered_controllers(),
    );
    // Desktop service management
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::service::all_service_registered_controllers(),
    );
    // Data migration utilities
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::migration::all_migration_registered_controllers(),
    );
    // Saved council definitions for the desktop Model Council surface.
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::council_registry::all_council_registry_registered_controllers(),
    );
    // Model Council: multi-model deliberation (parallel members + chair synthesis)
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::model_council::all_model_council_registered_controllers(),
    );
    // Background command monitors for agent-scoped event sources
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::monitor::all_monitor_registered_controllers(),
    );
    // Unified inference domain: text / vision / local runtime / cloud providers.
    // (Formerly split across inference, local AI, and providers modules.)
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::inference::all_inference_registered_controllers(),
    );
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::inference::all_local_inference_registered_controllers(),
    );
    // Embedding provider configuration and embed RPC.
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::embeddings::all_embeddings_registered_controllers(),
    );
    // People resolution and interaction scoring
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::people::all_people_registered_controllers(),
    );
    // Screen capture and UI analysis
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::screen_intelligence::all_screen_intelligence_registered_controllers(),
    );
    // Sandbox execution backends (Docker, local jail, policy, cleanup)
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::sandbox::all_sandbox_registered_controllers(),
    );
    // Backend Socket.IO bridge + related runtime plumbing
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::socket::all_socket_registered_controllers(),
    );
    // Managed Node.js runtime bridge (tool listing + dispatch)
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::javascript::all_javascript_registered_controllers(),
    );
    // Discovered SKILL.md skills and their bundled resources
    push(
        &mut controllers,
        DomainGroup::Skills,
        crate::openhuman::skills::all_skills_registered_controllers(),
    );
    // Skill runtime: run/cancel/log skill executions and resolve Node/Python toolchains
    push(
        &mut controllers,
        DomainGroup::Skills,
        crate::openhuman::skill_runtime::all_skill_runtime_registered_controllers(),
    );
    // Skill registry: browse, search, install from remote registries
    push(
        &mut controllers,
        DomainGroup::Skills,
        crate::openhuman::skill_registry::all_skill_registry_registered_controllers(),
    );
    // User workspace and file management
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::workspace::all_workspace_registered_controllers(),
    );
    // Workflow tool registry
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::tools::all_tools_registered_controllers(),
    );
    // Unified read-only registry across MCP stdio tools and controller-backed tools
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::tool_registry::all_tool_registry_registered_controllers(),
    );
    // Document and knowledge graph storage
    push(
        &mut controllers,
        DomainGroup::Memory,
        crate::openhuman::memory::all_memory_registered_controllers(),
    );
    // Long-term goals list (editable list + turn-based enrichment agent)
    push(
        &mut controllers,
        DomainGroup::Memory,
        crate::openhuman::memory_goals::all_memory_goals_registered_controllers(),
    );
    // Thread-level goal (Codex-style per-thread completion contract)
    push(
        &mut controllers,
        DomainGroup::Threads,
        crate::openhuman::thread_goals::all_thread_goals_registered_controllers(),
    );
    // Memory tree ingestion layer (#707 — canonicalised chunks with provenance)
    push(
        &mut controllers,
        DomainGroup::Memory,
        crate::openhuman::memory_tree::all_memory_tree_registered_controllers(),
    );
    // Memory tree retrieval layer (#710 — LLM-callable read tools over the tree)
    push(
        &mut controllers,
        DomainGroup::Memory,
        crate::openhuman::memory_tree::all_retrieval_registered_controllers(),
    );
    // Slack → memory-tree ingestion engine (per-message ingest, no bucketing)
    push(
        &mut controllers,
        DomainGroup::Memory,
        crate::openhuman::composio::providers::slack::all_slack_memory_registered_controllers(),
    );
    // Per-connection memory sync status, controls, and progress (#1136)
    push(
        &mut controllers,
        DomainGroup::Memory,
        crate::openhuman::memory_sync::sync_status::all_memory_sync_status_registered_controllers(),
    );
    // Memory sources — user-configured data connectors registry
    push(
        &mut controllers,
        DomainGroup::Memory,
        crate::openhuman::memory_sources::all_memory_sources_registered_controllers(),
    );
    // Memory diff — snapshot-based change tracking for memory sources
    push(
        &mut controllers,
        DomainGroup::Memory,
        crate::openhuman::memory_diff::all_memory_diff_registered_controllers(),
    );
    // Link shortener for long tracking URLs — saves LLM tokens
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::redirect_links::all_redirect_links_registered_controllers(),
    );
    // Referral and growth tracking
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::referral::all_referral_registered_controllers(),
    );
    // Billing and subscription management
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::billing::all_billing_registered_controllers(),
    );
    // Announcements surfaced on harness init
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::announcements::all_announcements_registered_controllers(),
    );
    // Team and role management
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::team::all_team_registered_controllers(),
    );
    // E2E test support — `openhuman.test_reset` wipes sidecar state in-place.
    // Gated behind the `e2e-test-support` cargo feature so shipped binaries
    // never even register the destructive wipe RPC. Flipped on by the E2E
    // build script (app/scripts/e2e-build.sh).
    #[cfg(feature = "e2e-test-support")]
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::test_support::all_test_support_registered_controllers(),
    );
    // Local wallet metadata and onboarding status
    push(
        &mut controllers,
        DomainGroup::Web3,
        crate::openhuman::wallet::all_wallet_registered_controllers(),
    );
    // High-level web3 surface (swaps / bridges / dapp calls) over the wallet
    push(
        &mut controllers,
        DomainGroup::Web3,
        crate::openhuman::web3::all_web3_registered_controllers(),
    );
    // Local assistive surfaces over third-party provider apps
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::provider_surfaces::all_provider_surfaces_registered_controllers(),
    );
    // OS-level text input interactions
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::text_input::all_text_input_registered_controllers(),
    );
    // Voice transcription and synthesis
    push(
        &mut controllers,
        DomainGroup::Voice,
        crate::openhuman::voice::all_voice_registered_controllers(),
    );
    // Background awareness and autonomous tasks
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::subconscious::all_subconscious_registered_controllers(),
    );
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::subconscious_triggers::all_subconscious_triggers_registered_controllers(),
    );
    // Webhook tunnel management
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::webhooks::all_webhooks_registered_controllers(),
    );
    // Core binary update management
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::update::all_update_registered_controllers(),
    );
    // Hierarchical knowledge summarization
    push(
        &mut controllers,
        DomainGroup::Memory,
        crate::openhuman::memory_tree::all_tree_summarizer_registered_controllers(),
    );
    // Self-learning and user context enrichment
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::learning::all_learning_registered_controllers(),
    );
    // Conversation thread and message management
    push(
        &mut controllers,
        DomainGroup::Threads,
        crate::openhuman::threads::all_threads_registered_controllers(),
    );
    // TokenJuice content-router debug controllers (detect / compress / cache_stats / retrieve).
    // Classified Platform (always-on): TokenJuice is the token-compression content
    // router that runs on every agent tool output, not a crypto surface — despite
    // #4802 listing it under the web3 gate. Flagged for #4802 re-scope.
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::tokenjuice::all_tokenjuice_registered_controllers(),
    );
    // Per-thread todo list (agent task board CRUD over RPC)
    push(
        &mut controllers,
        DomainGroup::Threads,
        crate::openhuman::todos::all_todos_registered_controllers(),
    );
    // Embedded webview native notifications
    push(
        &mut controllers,
        DomainGroup::Channels,
        crate::openhuman::webview_notifications::all_webview_notifications_registered_controllers(),
    );
    // Integration notification ingest, triage, and per-provider settings
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::notifications::all_notifications_registered_controllers(),
    );
    // Google Meet call-join request validation (shell handles the webview)
    push(
        &mut controllers,
        DomainGroup::Meet,
        crate::openhuman::meet::all_meet_registered_controllers(),
    );
    // Agent meetings — backend-delegated Meet bot via Socket.IO
    push(
        &mut controllers,
        DomainGroup::Meet,
        crate::openhuman::agent_meetings::all_agent_meetings_registered_controllers(),
    );
    // Live meet-agent loop: STT/LLM/TTS over the open call's audio.
    push(
        &mut controllers,
        DomainGroup::Meet,
        crate::openhuman::meet_agent::all_meet_agent_registered_controllers(),
    );
    // Desktop companion — Clicky-style interaction loop.
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::desktop_companion::all_desktop_companion_registered_controllers(),
    );
    // Structured WhatsApp Web data — agent-facing read-only controllers (list/search).
    // The write-path ingest controller is registered separately in build_internal_only_controllers.
    // Classified Channels (WhatsApp Web messaging surface) — not enumerated in the
    // spec Platform list; grouped with the other channel/webview domains.
    push(
        &mut controllers,
        DomainGroup::Channels,
        crate::openhuman::whatsapp_data::all_whatsapp_data_registered_controllers(),
    );
    // Mobile device pairing and management
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::devices::all_devices_registered_controllers(),
    );
    // Durable agent session database — queryable index over transcripts, lineage, tool calls
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::openhuman::session_db::all_session_db_registered_controllers(),
    );
    // One-time legacy session import into TinyAgents stores
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::openhuman::session_import::all_session_import_registered_controllers(),
    );
    // Background agent command center — read-only grouped view over the run ledger
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::openhuman::agent_orchestration::all_command_center_registered_controllers(),
    );
    // Durable dynamic workflow runs — definitions + read surface over the run ledger
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::openhuman::agent_orchestration::all_workflow_run_registered_controllers(),
    );
    // Durable agent-team coordination — teams, members, dependency-aware task claiming, messaging
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::openhuman::agent_orchestration::all_agent_team_registered_controllers(),
    );
    // Git-worktree isolation manager — list / status / diff / remove worker worktrees (#3376)
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::openhuman::agent_orchestration::all_worktree_registered_controllers(),
    );
    // User-driven cancel of detached background sub-agents (#3711)
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::openhuman::agent_orchestration::all_subagent_control_registered_controllers(),
    );
    controllers
}

/// Aggregates controllers that are registered for RPC routing but NOT exposed to agents.
///
/// These are write-path or internal-only handlers callable by trusted callers
/// (e.g. the Tauri scanner ingest path) that should not appear in agent tool listings.
fn build_internal_only_controllers() -> Vec<GroupedController> {
    let mut controllers = Vec::new();
    // whatsapp_data ingest: scanner-side write path.  Callable over RPC by the
    // Tauri scanner but excluded from agent-facing schema discovery.
    push(
        &mut controllers,
        DomainGroup::Channels,
        crate::openhuman::whatsapp_data::all_whatsapp_data_internal_controllers(),
    );
    // MCP write audit list: internal-only so the desktop UI/CLI can inspect
    // local write history without exposing cross-client history as an MCP tool.
    push(
        &mut controllers,
        DomainGroup::Mcp,
        crate::openhuman::mcp_audit::all_mcp_audit_internal_controllers(),
    );
    // tiny.place A2A social-network integration: renderer-callable via core_rpc_relay
    // but NOT advertised to agents in tool listings or schema discovery.
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::openhuman::tinyplace::all_tinyplace_registered_controllers(),
    );
    // User-consented tiny.place pairing for wrapped agent sessions: UI-callable
    // via core_rpc_relay, but excluded from agent tool listings/schema discovery.
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::openhuman::agent_orchestration::all_pairing_registered_controllers(),
    );
    // Orchestration read surface (stage 7): the TinyPlaceOrchestrationTab reads
    // sessions/messages, sends Master steering DMs, marks read, and polls status.
    // Renderer-only — not advertised to agents.
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::openhuman::orchestration::all_registered_controllers(),
    );
    controllers
}

/// Returns a vector of all currently registered controllers.
///
/// Filtered by the ambient [`crate::core::runtime::DomainSet`] (#4796): a
/// controller whose [`DomainGroup`] is disabled under the active context is
/// omitted. With no active context, or under `DomainSet::full()`, this returns
/// the complete set (byte-identical to pre-#4796).
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    registry()
        .iter()
        .filter(|g| group_allowed(g.group))
        .map(|g| g.controller.clone())
        .collect()
}

/// Returns a vector of all controller schemas, derived from the registered
/// controllers (the single source of truth). Kept identical in content to the
/// registered set — schemas can no longer drift from handlers (Phase 2).
///
/// Ambient-filtered by the active [`crate::core::runtime::DomainSet`] just like
/// [`all_registered_controllers`], so `/schema` omits gated namespaces
/// automatically under `harness()`.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    registry()
        .iter()
        .filter(|g| group_allowed(g.group))
        .map(|g| g.controller.schema.clone())
        .collect()
}

/// Generates a standardized RPC method name from a controller schema.
pub fn rpc_method_name(schema: &ControllerSchema) -> String {
    format!("openhuman.{}_{}", schema.namespace, schema.function)
}

/// Returns a human-readable description for a given namespace.
///
/// This is used for CLI help output.
pub fn namespace_description(namespace: &str) -> Option<&'static str> {
    match namespace {
        "about_app" => Some("Catalog the app's user-facing capabilities and where to find them."),
        "agentbox" => Some("AgentBox marketplace adapter status — mode flag and GMI MaaS provider wiring."),
        "ai" => Some("Agent-generated artifact storage, retrieval, and lifecycle management."),
        "app_state" => Some("Expose core-owned app shell state for frontend polling."),
        "auth" => Some("Manage app session and provider credentials."),
        "agent_experience" => Some("Local procedural experience capture and retrieval for agents."),
        "autocomplete" => Some("Inline autocomplete engine controls and style settings."),
        "channels" => Some("Channel definitions, connections, and lifecycle management."),
        "composio" => Some(
            "Composio OAuth integrations proxied via the backend — toolkits, connections, tools, and actions."
        ),
        "config" => Some("Read and update persisted runtime configuration."),
        "connectivity" => Some(
            "Connectivity diagnostics for the local sidecar, listening port, and backend Socket.IO state.",
        ),
        "cron" => Some("Manage scheduled jobs and run history."),
        "flows" => Some("Create, store, and run automation workflows."),
        "dashboard" => Some(
            "Operator-facing dashboard aggregations: per-model health comparison rows.",
        ),
        "mcp_clients" => Some(
            "Browse the Smithery.ai MCP registry, install MCP servers locally, manage their stdio connections, and expose their tools to the agent.",
        ),
        "mcp_setup" => Some(
            "MCP setup agent surface: search registries, request secrets out-of-band (opaque refs, no raw values in agent context), test, and install + connect.",
        ),
        "decrypt" => Some("Decrypt secure values managed by secret storage."),
        "doctor" => Some("Run diagnostics for workspace and runtime health."),
        "encrypt" => Some("Encrypt secure values managed by secret storage."),
        "health" => Some("Process and component health snapshots."),
        "inference" => Some("Connect to configured text, vision, and embedding inference runtimes."),
        "migrate" => Some("Data migration utilities."),
        "javascript" => Some("First-class JavaScript runtime bridge for listing and dispatching tools."),
        "monitor" => Some("Start, inspect, read, and stop bounded background command monitors."),
        "screen_intelligence" => Some("Screen capture, permissions, and accessibility automation."),
        "security" => Some("Security policy and autonomy guardrail metadata."),
        "service" => Some("Desktop service lifecycle management."),
        "session_import" => {
            Some("One-time import of legacy session transcripts into TinyAgents stores.")
        }
        "skill_registry" => Some("Browse, search, install, and uninstall skills from remote registries (OpenHuman, Hermes, OpenClaw)."),
        "skill_runtime" => Some("Run installed skills, inspect run logs, and resolve Node/Python skill runtimes."),
        "skills" => Some("Discovered SKILL.md skills (discovery, parse, install, run) and their resources."),
        "socket" => Some("Backend Socket.IO bridge controls."),
        "memory" => Some("Document storage, vector search, key-value store, and knowledge graph."),
        "memory_goals" => Some(
            "The agent's long-term goals list for working with the user — editable items plus turn-based enrichment.",
        ),
        "thread_goals" => Some(
            "The thread-level goal — a Codex-style per-thread completion contract with lifecycle, token budget, and idle continuation.",
        ),
        "memory_tree" => Some(
            "Canonical chunk ingestion, provenance capture, and chunk retrieval for source-grounded memory.",
        ),
        "memory_sync" => Some(
            "Per-connection memory sync status, user enable toggle, and live progress for the desktop UI.",
        ),
        "memory_sources" => Some(
            "User-configured data connectors (Composio, folders, GitHub repos, RSS, web pages) that feed memory.",
        ),
        "memory_diff" => Some(
            "Snapshot-based change tracking for memory sources — capture state, compute diffs, and surface changes to agents.",
        ),
        "redirect_links" => Some(
            "Shorten long tracking URLs to `openhuman://link/<id>` placeholders (SQLite-backed) to save tokens in prompts, with round-trip rewrite helpers.",
        ),
        "referral" => Some("Referral codes, stats, and apply flows via the hosted backend API."),
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
        "orchestration_pairing" => Some(
            "User-consented tiny.place contact pairing for wrapped agent sessions.",
        ),
        "orchestration" => Some(
            "Subconscious-orchestration read surface: chat windows (master/subconscious/per-session), message history, Master steering DMs, read state, and steering status.",
        ),
        "billing" => Some("Subscription plan, payment links, and credit top-up via the backend."),
        "announcements" => {
            Some("Latest active product announcement surfaced on harness init, via the backend.")
        }
        "team" => Some("Team member management, invites, and role changes via the backend."),
        "tool_registry" => Some(
            "Read-only discovery for MCP stdio tools and controller-backed tools, including routes, schemas, version, allowed agents, and health.",
        ),
        "test" => Some(
            "E2E test support — wipe sidecar state in-place between specs.",
        ),
        "wallet" => Some("Local wallet onboarding status and derived multi-chain account metadata."),
        "web3_swap" => Some("Single-chain crypto swaps via deBridge, built on the local wallet."),
        "web3_bridge" => Some("Cross-chain crypto bridges via deBridge DLN, built on the local wallet."),
        "web3_dapp" => Some("Generic EVM dapp contract calls signed by the local wallet."),
        "provider_surfaces" => Some(
            "Local-first assistive surfaces for provider events, respond queues, and drafts.",
        ),
        "voice" => Some("Speech-to-text and text-to-speech using local models."),
        "subconscious" => Some("Periodic local-model background awareness loop."),
        "subconscious_triggers" => {
            Some("Event-driven trigger pipeline feeding the background orchestrator.")
        }
        "text_input" => Some("Read, insert, and preview text in the OS-focused input field."),
        "webhooks" => {
            Some("Webhook tunnel registrations and captured request/response debug logs.")
        }
        "webview_apis" => Some(
            "Typed connector APIs (Gmail, …) proxied over a loopback WebSocket to the Tauri shell so core-side JSON-RPC reaches live-webview CDP operations.",
        ),
        "update" => {
            Some("Self-update: check GitHub Releases for newer core binary and stage updates.")
        }
        "tree_summarizer" => {
            Some("Hierarchical time-based summarization tree for background knowledge compression.")
        }
        "learning" => Some(
            "User context enrichment — LinkedIn profile scraping and onboarding intelligence.",
        ),
        "people" => {
            Some("Contact resolution and recency × frequency × reciprocity × depth scoring.")
        },
        "notification" => Some(
            "Integration notification ingest, triage scoring, listing, read-state, \
             and per-provider routing settings.",
        ),
        "meet" => Some(
            "Validate Google Meet call-join requests and mint a request_id; the desktop \
             shell opens the embedded CEF webview that joins the call as an anonymous guest. \
             Also provides meet_list_upcoming to fetch upcoming calendar meetings with \
             conferencing links from connected Google Calendar accounts.",
        ),
        "meet_agent" => Some(
            "Live agent loop for an open Google Meet call: shell streams inbound PCM, \
             core runs VAD-segmented STT → LLM → TTS, shell pulls synthesized PCM back.",
        ),
        "agent_meetings" => Some(
            "Backend-delegated meeting bot (Google Meet, Zoom, Teams, Webex) via Socket.IO — join, leave, and harness response.",
        ),
        "devices" => Some(
            "Paired mobile device management — pairing channel creation, listing, and revocation.",
        ),
        "whatsapp_data" => Some(
            "Structured WhatsApp conversation and message store — list chats, read messages, and search across WhatsApp Web data.",
        ),
        "companion" => Some(
            "Desktop companion — Clicky-style hotkey-driven interaction loop with STT, LLM, TTS, and visual pointing.",
        ),
        "tinyplace" => Some(
            "tiny.place A2A social-network integration: directory, explorer, and search over the agent network.",
        ),
        _ => None,
    }
}

/// Returns the CLI handler for a given namespace, if one is registered.
pub fn cli_handler_for_namespace(namespace: &str) -> Option<CliHandler> {
    cli_adapters()
        .iter()
        .find(|a| a.namespace == namespace)
        .map(|a| a.handler)
}

/// Looks up an RPC method name based on namespace and function.
pub fn rpc_method_from_parts(namespace: &str, function: &str) -> Option<String> {
    // Searches the FULL (unfiltered) registry: this backs parameter validation
    // and CLI routing, which are harmless for an about-to-be-rejected gated
    // method — the DomainSet gate is enforced at dispatch
    // (`try_invoke_registered_rpc`), not here. See that fn for the rationale.
    registry()
        .iter()
        .find(|g| {
            g.controller.schema.namespace == namespace && g.controller.schema.function == function
        })
        .map(|g| g.controller.rpc_method_name())
}

/// Retrieves the schema for a specific RPC method.
///
/// Checks both the agent-facing registry and the internal registry so that
/// parameter validation still applies to internal-only methods (e.g. ingest).
pub fn schema_for_rpc_method(method: &str) -> Option<ControllerSchema> {
    // DomainSet gate (#4796): a method whose group is disabled under the ambient
    // context must be indistinguishable from a genuinely-unregistered method at
    // EVERY public lookup, not just at dispatch. Filtering here (identically to
    // `try_invoke_registered_rpc`) means `invoke_method_inner` never runs param
    // validation against a gated method — otherwise a gated `openhuman.flows_*`
    // call with bad params would return the controller's validation error
    // instead of method-not-found, leaking the hidden RPC surface. No ambient
    // context ⇒ `group_allowed` is `true` ⇒ unfiltered, identical to pre-#4796.
    registry()
        .iter()
        .chain(internal_registry().iter())
        .find(|g| g.controller.rpc_method_name() == method && group_allowed(g.group))
        .map(|g| g.controller.schema.clone())
}

/// Validates that the provided parameters match the requirements of the controller schema.
///
/// # Errors
///
/// Returns an error message if required parameters are missing or if unknown parameters are provided.
pub fn validate_params(
    schema: &ControllerSchema,
    params: &Map<String, Value>,
) -> Result<(), String> {
    for input in &schema.inputs {
        if input.required && !params.contains_key(input.name) {
            return Err(format!(
                "missing required param '{}': {}",
                input.name, input.comment
            ));
        }
    }

    for key in params.keys() {
        if !schema.inputs.iter().any(|f| f.name == key) {
            return Err(format!(
                "unknown param '{}' for {}.{}",
                key, schema.namespace, schema.function
            ));
        }
    }

    // Type-check each present param against its declared `TypeSchema`, so every
    // controller gets uniform validation before dispatch rather than relying on
    // each handler's `serde_json::from_value`. Absent (optional) params are
    // already handled by the required-presence check above.
    for input in &schema.inputs {
        if let Some(value) = params.get(input.name) {
            check_type(value, &input.ty).map_err(|expected| {
                format!(
                    "invalid type for param '{}' in {}.{}: expected {}, got {}",
                    input.name,
                    schema.namespace,
                    schema.function,
                    expected,
                    json_type_name(value),
                )
            })?;
        }
    }

    Ok(())
}

/// A short, human-readable name for the JSON kind of `value`, used in
/// `validate_params` type-mismatch errors.
fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Validate a JSON `value` against a declared [`TypeSchema`].
///
/// Returns `Ok(())` on a match, or `Err(expected)` where `expected` is a short
/// description of the type that was required. Unknown/opaque shapes
/// (`Json`, `Bytes`, `Ref`) accept any value — they are validated by the
/// handler's typed deserialization.
fn check_type(value: &Value, ty: &crate::core::TypeSchema) -> Result<(), &'static str> {
    use crate::core::TypeSchema;

    // JSON-RPC semantics (preserved from the prior presence-only check):
    // an explicit `null` satisfies validation for any declared type. Required
    // fields are checked for *presence*, not value; stronger contracts are
    // enforced by the handler's typed deserialization.
    if value.is_null() {
        return Ok(());
    }

    match ty {
        // Opaque / handler-validated shapes accept any JSON value.
        //
        // Structured types (`Object`/`Map`/`Ref`) are deliberately lenient: a
        // struct field may have a custom `Deserialize` impl that accepts more
        // than one JSON shape (e.g. `agent_registry.update`'s `subagents`
        // accepts both `{ "allowlist": [...] }` and a legacy bare array), and
        // the declared schema can only describe one of them. Strictly checking
        // the JSON kind here would reject inputs the handler's `serde_json`
        // deserialization accepts. We therefore keep pre-dispatch validation to
        // scalar leaf types (where a type confusion would otherwise reach an
        // `as_str()/as_u64()`-style accessor) and defer object/map shape
        // validation to the handler.
        TypeSchema::Json
        | TypeSchema::Bytes
        | TypeSchema::Ref(_)
        | TypeSchema::Object { .. }
        | TypeSchema::Map(_) => Ok(()),

        TypeSchema::Bool => value.is_boolean().then_some(()).ok_or("bool"),
        TypeSchema::String => value.is_string().then_some(()).ok_or("string"),
        TypeSchema::I64 => value.is_i64().then_some(()).ok_or("integer"),
        TypeSchema::U64 => value.is_u64().then_some(()).ok_or("unsigned integer"),
        TypeSchema::F64 => {
            // Accept any JSON number (ints are valid floats).
            value.is_number().then_some(()).ok_or("number")
        }

        // `Option<T>` accepts null or a value matching the inner type.
        TypeSchema::Option(inner) => {
            if value.is_null() {
                Ok(())
            } else {
                check_type(value, inner)
            }
        }

        TypeSchema::Array(inner) => match value.as_array() {
            Some(items) => {
                for item in items {
                    check_type(item, inner)?;
                }
                Ok(())
            }
            None => Err("array"),
        },

        TypeSchema::Enum { variants } => match value.as_str() {
            Some(s) if variants.contains(&s) => Ok(()),
            Some(_) => Err("one of the allowed enum variants"),
            None => Err("string"),
        },
    }
}

/// Attempts to invoke a registered RPC method by name.
///
/// Checks both the agent-facing controller registry and the internal-only registry,
/// so scanner-side write paths (e.g. `openhuman.whatsapp_data_ingest`) are routable
/// even though they are not included in agent tool listings.
///
/// Returns `None` if the method is not found in either registry.
pub async fn try_invoke_registered_rpc(
    method: &str,
    params: Map<String, Value>,
) -> Option<Result<Value, String>> {
    let grouped = registry()
        .iter()
        .chain(internal_registry().iter())
        .find(|g| g.controller.rpc_method_name() == method)?;

    // DomainSet gate (#4796): a method whose group is disabled under the ambient
    // context reports as an unknown method (`None`) — the same result a caller
    // gets for a genuinely-unregistered method, so a gated domain's controllers
    // are indistinguishable from absent. Enforced HERE (dispatch), not in
    // schema/validation lookups, to avoid a validate/dispatch split.
    if !group_allowed(grouped.group) {
        log::debug!(
            "[rpc][domain-gate] method '{method}' suppressed — group {:?} disabled under active DomainSet",
            grouped.group
        );
        return None;
    }
    let handler = grouped.controller.handler;

    // Establish the ambient CoreContext for the duration of the handler so
    // `CoreContext::current()` resolves inside handler bodies (Phase 2).
    // `current()` inherits an already-active scope (so a handler that dispatches
    // a nested RPC stays in the same tenant context) and otherwise resolves to
    // the process default. Before any context is built (some unit tests) it is
    // `None` and the handler runs unscoped.
    //
    // The scoped future is re-boxed back into a `ControllerFuture` so the
    // concrete `TaskLocalFuture` type does not escape into this fn's future —
    // without the type erasure the `Send` auto-trait solver overflows on the
    // largest handler futures (E0275).
    use crate::core::runtime::context::CoreContext;
    let fut = handler(params);
    let scoped: ControllerFuture = match CoreContext::current() {
        Some(ctx) => Box::pin(CoreContext::scope(ctx, fut)),
        None => fut,
    };
    Some(scoped.await)
}

/// Validates the consistency of the controller registry.
///
/// The registry is the single source of truth: each [`RegisteredController`]
/// carries its own schema, and the public schema list is *derived* from it
/// (see [`all_controller_schemas`]). There is therefore no separate "declared"
/// list to drift from — the previous declared-vs-registered cross-check is
/// impossible by construction and has been removed (Phase 2 registry collapse).
///
/// Ensures that:
/// - There are no duplicate controllers or RPC methods.
/// - Namespaces and functions are not empty.
/// - Required input names are unique within a controller.
fn validate_registry(registered: &[GroupedController]) -> Result<(), String> {
    use std::collections::{BTreeMap, BTreeSet};

    let mut errors: Vec<String> = Vec::new();
    let mut registered_keys = BTreeSet::new();
    let mut registered_rpc_methods = BTreeSet::new();

    for grouped in registered {
        let controller = &grouped.controller;
        let schema = &controller.schema;
        let key = format!("{}.{}", schema.namespace, schema.function);
        if !registered_keys.insert(key.clone()) {
            errors.push(format!("duplicate registered controller `{key}`"));
        }

        let rpc_method = controller.rpc_method_name();
        if !registered_rpc_methods.insert(rpc_method.clone()) {
            errors.push(format!("duplicate registered rpc method `{rpc_method}`"));
        }

        if schema.namespace.trim().is_empty() {
            errors.push(format!(
                "invalid registered controller `{key}`: namespace must not be empty"
            ));
        }
        if schema.function.trim().is_empty() {
            errors.push(format!(
                "invalid registered controller `{key}`: function must not be empty"
            ));
        }

        let mut required_inputs = BTreeSet::new();
        let mut required_dupes: BTreeMap<String, usize> = BTreeMap::new();
        for input in schema.inputs.iter().filter(|input| input.required) {
            if !required_inputs.insert(input.name.to_string()) {
                *required_dupes.entry(input.name.to_string()).or_default() += 1;
            }
        }
        for (name, _) in required_dupes {
            errors.push(format!(
                "duplicate required input `{name}` in `{}`",
                schema.method_name()
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[derive(Debug, Clone)]
pub struct HttpMethodSchemaDefinition {
    pub method: String,
    pub namespace: &'static str,
    pub function: &'static str,
    pub description: &'static str,
    pub inputs: Vec<crate::core::FieldSchema>,
    pub outputs: Vec<crate::core::FieldSchema>,
}

pub fn all_http_method_schemas() -> Vec<HttpMethodSchemaDefinition> {
    let mut methods = vec![
        HttpMethodSchemaDefinition {
            method: "core.ping".to_string(),
            namespace: "core",
            function: "ping",
            description: "Liveness probe for the core JSON-RPC server.",
            inputs: vec![],
            outputs: vec![crate::core::FieldSchema {
                name: "ok",
                ty: crate::core::TypeSchema::Bool,
                comment: "Always true when the server is reachable.",
                required: true,
            }],
        },
        HttpMethodSchemaDefinition {
            method: "core.version".to_string(),
            namespace: "core",
            function: "version",
            description: "Returns the core binary version.",
            inputs: vec![],
            outputs: vec![crate::core::FieldSchema {
                name: "version",
                ty: crate::core::TypeSchema::String,
                comment: "Semantic version string for the running core binary.",
                required: true,
            }],
        },
    ];
    methods.extend(
        all_controller_schemas()
            .into_iter()
            .map(|schema| HttpMethodSchemaDefinition {
                method: rpc_method_name(&schema),
                namespace: schema.namespace,
                function: schema.function,
                description: schema.description,
                inputs: schema.inputs,
                outputs: schema.outputs,
            }),
    );
    methods
}

/// Shared test helper: assert a domain's controller-schema list and
/// registered-controller list stay in lockstep, and that a known function is
/// present. Replaces the brittle `assert_eq!(schemas().len(), N)` magic-number
/// pattern repeated across ~15 domains (plan.md §3/§6) — a legitimate new
/// controller no longer breaks the count, but a schema/handler desync or a
/// dropped op still fails.
#[cfg(test)]
pub(crate) fn assert_schema_controller_parity(
    schemas: &[ControllerSchema],
    controllers: &[RegisteredController],
    known_function: &str,
) {
    assert_eq!(
        schemas.len(),
        controllers.len(),
        "schema/controller registration lists must stay in lockstep",
    );
    assert!(
        schemas.iter().any(|s| s.function == known_function),
        "expected a `{known_function}` controller schema to be registered",
    );
}

#[cfg(test)]
#[path = "all_tests.rs"]
mod tests;
