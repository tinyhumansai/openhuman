//! Registry and dispatch logic for all OpenHuman controllers.
//!
//! This module serves as the central hub for registering domain-specific
//! controllers (e.g., memory, skills, config) and providing a unified
//! interface for both the CLI and RPC layers to invoke them.

use std::sync::{Arc, OnceLock, RwLock};

use serde_json::{Map, Value};

#[path = "all_entry.rs"]
mod all_entry;
pub use all_entry::{
    BoundRegisteredController, CliHandler, ControllerEntry, ControllerFuture, ControllerHandler,
    ControllerTask, RegisteredCliAdapter, RegisteredController,
};

use crate::core::ControllerSchema;

pub use super::domain_group::DomainGroup;
mod components_gate;
mod namespaces;
mod validate;
pub use components_gate::disabled_component_for_method;
pub use namespaces::namespace_description;
use validate::validate_registry;

/// A [`RegisteredController`] tagged with the [`DomainGroup`] it belongs to.
///
/// The registry stores these so the live surface can be filtered by the ambient
/// [`crate::core::runtime::DomainSet`] WITHOUT touching the ~109 domain modules'
/// bare `RegisteredController { schema, handler }` literals — the group tag is
/// attached once, here, at the single registration site.
#[derive(Clone)]
struct GroupedController {
    group: DomainGroup,
    controller: ControllerEntry,
}

/// Append `items` to `dst`, tagging each with `group`.
/// This is the single seam that attaches a [`DomainGroup`] to every domain's
/// controllers without the domain modules knowing about groups.
fn push(dst: &mut Vec<GroupedController>, group: DomainGroup, items: Vec<RegisteredController>) {
    dst.extend(items.into_iter().map(|controller| GroupedController {
        group,
        controller: ControllerEntry::legacy(controller),
    }));
}

fn push_entries(dst: &mut Vec<GroupedController>, group: DomainGroup, items: Vec<ControllerEntry>) {
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

/// Whether the given [`DomainGroup`] is enabled under the ambient [`DomainSet`].
fn group_allowed(group: DomainGroup) -> bool {
    active_domain_set().map_or(group != DomainGroup::Operator, |s| s.allows(group))
}

/// Whether `g` is live in the ambient scope.
fn visible(g: &GroupedController) -> bool {
    let doctor_report = g.controller.rpc_method_name() == "openhuman.doctor_report";
    // Only the shared report bypasses its group gate; surface checks still apply.
    let saas_doctor_report = crate::core::runtime::is_saas() && doctor_report;
    // That surface check denies missing scopes and unrelated Platform methods.
    (group_allowed(g.group) || saas_doctor_report) && on_surface(g)
}

fn on_surface(g: &GroupedController) -> bool {
    let operator = g.group == DomainGroup::Operator;
    crate::profiles::surface::method_visible(&g.controller.rpc_method_name(), operator)
}

/// The global static registry of all controllers, initialized once on first access.
static REGISTRY: OnceLock<Vec<GroupedController>> = OnceLock::new();

/// Internal-only controllers: registered for RPC dispatch but NOT in the agent-facing
/// schema catalog.  These handlers are callable by trusted callers (e.g. the desktop shell)
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

/// Controllers registered by a crate *above* the core at runtime — today the
/// hosted TinyHumans proxies (`billing`, `team`, `referral`, `announcements`)
/// from `openhuman-tinyhumans`, which the core cannot name because the core
/// carries no backend client.
///
/// Append-only and chained into every lookup below alongside [`registry`],
/// so there is no "register before first registry access" ordering rule: a
/// host may install an extension before or after the core boots, and the
/// only observable rule is "install before the first dispatch of one of its
/// methods". [`DomainSet`](crate::core::runtime::DomainSet) gating applies to
/// extension controllers exactly as to built-in ones through their group.
static EXTENSIONS: RwLock<Option<Arc<Vec<GroupedController>>>> = RwLock::new(None);
static EXTENSION_NAMESPACES: RwLock<Vec<(&'static str, &'static str)>> = RwLock::new(Vec::new());

/// A set of controllers a crate above the core contributes to the registry.
pub struct ControllerExtension {
    /// The domain family every controller in `controllers` belongs to; the
    /// ambient [`DomainSet`](crate::core::runtime::DomainSet) gates them
    /// through it.
    pub group: DomainGroup,
    /// The controllers, built exactly as a core domain builds them.
    pub controllers: Vec<RegisteredController>,
    /// `(namespace, description)` pairs for [`namespace_description`].
    pub namespaces: &'static [(&'static str, &'static str)],
}

/// Register `ext`'s controllers alongside the built-in registry.
///
/// Idempotent for an identical re-registration (same `(namespace, function)`
/// keys already present): returns `Ok(())` and changes nothing, so a host or
/// test fixture may call it freely. Any *other* collision with the built-in,
/// internal or previously extended set is an error, checked with the same
/// [`validate_registry`] drift guard the built-in set passes at boot.
pub fn register_controller_extension(ext: ControllerExtension) -> Result<(), String> {
    let mut slot = EXTENSIONS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let existing: Vec<GroupedController> = slot.as_deref().cloned().unwrap_or_default();

    let incoming_keys: Vec<String> = ext
        .controllers
        .iter()
        .map(|c| format!("{}.{}", c.schema.namespace, c.schema.function))
        .collect();
    let already: std::collections::BTreeSet<String> = existing
        .iter()
        .map(|g| {
            format!(
                "{}.{}",
                g.controller.schema.namespace, g.controller.schema.function
            )
        })
        .collect();
    if !incoming_keys.is_empty() && incoming_keys.iter().all(|k| already.contains(k)) {
        log::debug!(
            "[registry] extension for group {:?} already registered ({} controllers); no-op",
            ext.group,
            incoming_keys.len()
        );
        return Ok(());
    }

    let mut merged = existing;
    for controller in ext.controllers {
        merged.push(GroupedController {
            group: ext.group,
            controller: ControllerEntry::legacy(controller),
        });
    }
    // Validate the union so an extension cannot shadow a built-in or internal
    // method, and cannot carry an invalid schema the boot guard would reject.
    let union: Vec<GroupedController> = registry()
        .iter()
        .chain(internal_registry().iter())
        .chain(merged.iter())
        .cloned()
        .collect();
    validate_registry(&union).map_err(|err| format!("invalid controller extension: {err}"))?;

    log::info!(
        "[registry] registered controller extension group={:?} controllers={} namespaces={:?}",
        ext.group,
        incoming_keys.len(),
        ext.namespaces.iter().map(|(n, _)| *n).collect::<Vec<_>>()
    );
    *slot = Some(Arc::new(merged));
    let mut names = EXTENSION_NAMESPACES
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for pair in ext.namespaces {
        if !names.iter().any(|(n, _)| n == &pair.0) {
            names.push(*pair);
        }
    }
    Ok(())
}

/// Snapshot of the extension registry: an `Arc` clone per lookup, never a
/// `Vec` clone.
fn extension_registry() -> Arc<Vec<GroupedController>> {
    EXTENSIONS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .unwrap_or_else(|| Arc::new(Vec::new()))
}

/// The agent-facing registry: built-in controllers followed by every
/// registered extension. Every public lookup iterates this, so extension
/// controllers are first-class for schema, dispatch, capability and CLI
/// routing.
struct RegistryView {
    builtin: &'static [GroupedController],
    extensions: Arc<Vec<GroupedController>>,
}

impl RegistryView {
    fn iter(&self) -> impl Iterator<Item = &GroupedController> + '_ {
        self.builtin.iter().chain(self.extensions.iter())
    }
}

fn registry_view() -> RegistryView {
    RegistryView {
        builtin: registry(),
        extensions: extension_registry(),
    }
}

/// Description registered by an extension for `namespace`, if any.
fn extension_namespace_description(namespace: &str) -> Option<&'static str> {
    EXTENSION_NAMESPACES
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .find(|(n, _)| *n == namespace)
        .map(|(_, d)| *d)
}

/// Returns a reference to the global CLI adapter registry.
fn cli_adapters() -> &'static [RegisteredCliAdapter] {
    CLI_ADAPTERS.get_or_init(|| {
        vec![
            // Bare `openhuman subsystems` prints the slot table; `openhuman
            // subsystems status` still routes through the generic namespace
            // dispatcher and prints JSON.
            RegisteredCliAdapter {
                namespace: "subsystems",
                handler: crate::core::subsystems_cli::run_subsystems_command,
            },
        ]
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
        crate::platform::about_app::all_about_app_registered_controllers(),
    );
    // Core application shell state
    push_entries(
        &mut controllers,
        DomainGroup::Desktop,
        crate::desktop::app_state::all_app_state_controller_entries(),
    );
    // Composio integration controllers
    push_entries(
        &mut controllers,
        DomainGroup::Integrations,
        crate::integrations::composio::all_composio_controller_entries(),
    );
    // Scheduled job management
    push(
        &mut controllers,
        DomainGroup::Automation,
        crate::cron::all_cron_registered_controllers(),
    );
    // Saved automation workflows (tinyflows graphs): create/get/list/update/delete/run
    // (gated with flows).
    #[cfg(feature = "flows")]
    push(
        &mut controllers,
        DomainGroup::Flows,
        crate::flows::all_flows_registered_controllers(),
    );
    // Inbound event queues: depths, dead letters, redrive (`[components]
    // eventqueues`; the handlers answer COMPONENT_DISABLED when it is off).
    #[cfg(feature = "inbound-queues")]
    push(
        &mut controllers,
        DomainGroup::Automation,
        crate::inbound::all_inbound_registered_controllers(),
    );
    // Proactive task ingestion from external tools (github/notion/linear/clickup)
    push(
        &mut controllers,
        DomainGroup::Integrations,
        crate::integrations::task_sources::all_task_sources_registered_controllers(),
    );
    push(
        &mut controllers,
        DomainGroup::Desktop,
        crate::desktop::dashboard::all_dashboard_registered_controllers(),
    );
    push_entries(
        &mut controllers,
        DomainGroup::Mcp,
        crate::mcp::all_controller_entries(),
    );
    // Agent definition and prompt inspection
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::agent_runs::all_agent_registered_controllers(),
    );
    // Read-only agent run replay + status over the durable journal/status seams
    // (agent_run_events / agent_run_status / agent_runs_active).
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::agent_runs::replay::all_agent_replay_registered_controllers(),
    );
    // User-facing agent registry: defaults, enablement, custom agents, tool policy.
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::agent_registry::all_agent_registry_registered_controllers(),
    );
    // System and process health monitoring
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::platform::health::all_health_registered_controllers(),
    );
    // Kernel subsystem/driver bindings: slot, bound driver, class, health,
    // contract version, capabilities (docs/specs/kernel.md §6 item 6). The one
    // controller registered from `crates/openhuman-core/src/core/` — it is a kernel binding table
    // with no `crates/openhuman-core/src/` family of its own, so it is tagged `Platform`
    // rather than earning a `DomainGroup` variant for a single read-only
    // function. Consequence: like `health`, it is absent under
    // `DomainSet::harness()`, while `memory.provider_status` (a `Memory`
    // family method) stays reachable there.
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::core::subsystem::all_subsystems_registered_controllers(),
    );
    // Diagnostic tools
    push_entries(
        &mut controllers,
        DomainGroup::Platform,
        crate::doctor::all_doctor_controller_entries(),
    );
    // User-authored hooks — inspect, reload, and test-fire `hooks.json` entries.
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::hooks::all_hooks_registered_controllers(),
    );
    // Secret storage and encryption
    push_entries(
        &mut controllers,
        DomainGroup::Security,
        crate::security::encryption::all_encryption_controller_entries(),
    );
    // Keyring consent — user approval before local secret storage fallback
    push_entries(
        &mut controllers,
        DomainGroup::Security,
        crate::security::keyring_consent::all_keyring_consent_controller_entries(),
    );
    // Security policy metadata
    push(
        &mut controllers,
        DomainGroup::Security,
        crate::security::all_security_registered_controllers(),
    );
    // Interactive approval workflow (#1339 — gate external-effect tool calls)
    push_entries(
        &mut controllers,
        DomainGroup::Security,
        crate::security::approval::schemas::all_approval_controller_entries(),
    );
    // Interactive plan-review gate — parks a live turn on a thread-scoped plan
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::plan_review::all_plan_review_registered_controllers(),
    );
    // Per-thread Plan/Build run mode (agent.set_run_mode / agent.get_run_mode)
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::threads::run_mode::all_registered_controllers(),
    );
    // Agent-generated artifact storage, retrieval, and lifecycle management
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::artifacts::all_artifacts_registered_controllers(),
    );
    // Read-only command palette listing: built-ins merged with skills.list /
    // flows.list (C5). Tagged `Agent` rather than a new `DomainGroup` variant
    // — it is chat-harness surface, always on, and adding a variant for this
    // single-RPC domain would touch every exhaustive `DomainGroup` match in
    // this file.
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::commands::all_commands_registered_controllers(),
    );
    // Token usage and billing cost tracking
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::platform::cost::all_cost_registered_controllers(),
    );
    // x402 machine-payable API payment protocol
    push(
        &mut controllers,
        DomainGroup::Web3,
        crate::web3::x402::all_x402_registered_controllers(),
    );
    // External messaging channels (Web, Telegram, etc.)
    push(
        &mut controllers,
        DomainGroup::Channels,
        crate::web_chat::all_web_channel_registered_controllers(),
    );
    // External messaging channels (Telegram, Discord, Slack, …).
    // Gated behind the `channels` feature. NOTE: the web_chat push above stays
    // ungated — the in-app chat is core product surface (decoupled in #5002),
    // even though its runtime tag is DomainGroup::Channels.
    #[cfg(feature = "channels")]
    push(
        &mut controllers,
        DomainGroup::Channels,
        crate::channels::controllers::all_channels_registered_controllers(),
    );
    // Persistent configuration management
    push(
        &mut controllers,
        DomainGroup::Config,
        crate::config::all_config_registered_controllers(),
    );
    // Local sidecar reachability + backend Socket.IO state diagnostics (#1527)
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::platform::connectivity::all_connectivity_registered_controllers(),
    );
    // User credentials and session management
    push(
        &mut controllers,
        DomainGroup::Security,
        crate::credentials::all_credentials_registered_controllers(),
    );
    // Desktop service management
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::platform::service::all_service_registered_controllers(),
    );
    let mut inference_controllers = crate::inference::all_inference_registered_controllers();
    // Agent-owned chat keeps inference's capability group and contract order.
    // Append it before the local runtime's speech controllers.
    // This remains the registry's sole registration site for these methods.
    inference_controllers
        .extend(crate::agent_runs::all_inference_agent_chat_registered_controllers());
    inference_controllers.extend(crate::inference::all_local_inference_registered_controllers());
    push(
        &mut controllers,
        DomainGroup::Inference,
        inference_controllers,
    );
    // Embedding provider configuration and embed RPC.
    push(
        &mut controllers,
        DomainGroup::Inference,
        crate::inference::embedding_host::all_embeddings_registered_controllers(),
    );
    // Sandbox execution backends (Docker, local jail, policy, cleanup)
    push(
        &mut controllers,
        DomainGroup::Runtimes,
        crate::sandbox::all_sandbox_registered_controllers(),
    );
    // Backend Socket.IO bridge + related runtime plumbing
    push_entries(
        &mut controllers,
        DomainGroup::Platform,
        crate::platform::socket::all_controller_entries(),
    );
    // Discovered SKILL.md skills and their bundled resources
    push(
        &mut controllers,
        DomainGroup::Skills,
        crate::skills::all_skills_registered_controllers(),
    );
    // Skill runtime: run/cancel/log skill executions and resolve Node/Python toolchains
    push(
        &mut controllers,
        DomainGroup::Skills,
        crate::skills::runtime::all_skill_runtime_registered_controllers(),
    );
    // Skill registry: browse, search, install from remote registries
    push(
        &mut controllers,
        DomainGroup::Skills,
        crate::skills::catalog::all_skill_registry_registered_controllers(),
    );
    // User workspace and file management
    push(
        &mut controllers,
        DomainGroup::Config,
        crate::config::workspace::all_workspace_registered_controllers(),
    );
    // Workflow tool registry
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::tools::all_tools_registered_controllers(),
    );
    // Unified read-only registry across MCP stdio tools and controller-backed tools
    push_entries(
        &mut controllers,
        DomainGroup::Platform,
        crate::tools::registry::all_tool_registry_bound_controller_entries(),
    );
    // Memory v2 (`docs/specs/memory-v2.md`): engines, recall/fetch/learn/
    // forget, conversations, document sources, context.md and v1 import.
    // Always registered: with no usable engine the methods answer MEMORY_OFF
    // (and the settings/import ones still work), so the UI can explain why.
    push(
        &mut controllers,
        DomainGroup::Memory,
        crate::memory::all_memory_registered_controllers(),
    );
    // The hosted TinyHumans proxies (`billing`, `team`, `referral`,
    // `announcements`, `DomainGroup::Hosted`) are NOT built in: they live in
    // `openhuman-tinyhumans` and arrive through `register_controller_extension`
    // when a host installs that crate. A core without it has no such RPCs.
    // E2E test support — `openhuman.test_reset` wipes sidecar state in-place.
    // Gated behind the `e2e-test-support` cargo feature so shipped binaries
    // never even register the destructive wipe RPC. Flipped on by the E2E
    // build script (app/scripts/e2e-build.sh).
    #[cfg(feature = "e2e-test-support")]
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::test_support::all_test_support_registered_controllers(),
    );
    // Local wallet metadata and onboarding status
    push(
        &mut controllers,
        DomainGroup::Web3,
        crate::web3::wallet::all_wallet_registered_controllers(),
    );
    // High-level web3 surface (swaps / bridges / dapp calls) over the wallet
    push(
        &mut controllers,
        DomainGroup::Web3,
        crate::web3::all_web3_registered_controllers(),
    );
    // Local assistive surfaces over third-party provider apps
    push(
        &mut controllers,
        DomainGroup::Desktop,
        crate::desktop::provider_surfaces::all_provider_surfaces_registered_controllers(),
    );
    // Voice transcription and synthesis (gated behind the `voice` feature).
    #[cfg(feature = "voice")]
    push_entries(
        &mut controllers,
        DomainGroup::Voice,
        crate::voice::all_voice_controller_entries(),
    );
    // Webhook tunnel management
    push(
        &mut controllers,
        DomainGroup::Skills,
        crate::skills::webhooks::all_webhooks_registered_controllers(),
    );
    // Core binary update management
    push(
        &mut controllers,
        DomainGroup::Platform,
        crate::platform::update::all_update_registered_controllers(),
    );
    // Conversation thread and message management
    push_entries(
        &mut controllers,
        DomainGroup::Threads,
        crate::threads::schemas::all_controller_entries(),
    );
    // TokenJuice content-router debug controllers (detect / compress / cache_stats / retrieve).
    // Classified Inference: TokenJuice is the token-compression content router,
    // not a crypto surface — despite #4802 listing it under the web3 gate.
    // Only these debug/inspection controllers are gated; the content-router
    // subscriber used on agent tool output remains always-on core infra.
    push(
        &mut controllers,
        DomainGroup::Inference,
        crate::inference::tokenjuice::all_tokenjuice_registered_controllers(),
    );
    // Integration notification ingest, triage, and per-provider settings
    push(
        &mut controllers,
        DomainGroup::Desktop,
        crate::desktop::notifications::all_notifications_registered_controllers(),
    );
    // Storage layout v1 -> v2 migration, status and restore
    #[cfg(feature = "legacy-import")]
    {
        #[rustfmt::skip]
    push(&mut controllers, DomainGroup::Config, crate::storage::layout::schemas::all_registered_controllers());
    }
    // Mobile device pairing and management
    push_entries(
        &mut controllers,
        DomainGroup::Security,
        crate::security::devices::all_controller_entries(),
    );
    // Durable agent/workflow run ledger — read surface over run state, lineage, events, telemetry
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::sessions::all_session_db_registered_controllers(),
    );
    // One-time legacy session import into TinyAgents stores
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::storage::import::all_session_import_registered_controllers(),
    );
    // Background agent command center, workflow runs, teams, worktrees, and subagent controls.
    push(
        &mut controllers,
        DomainGroup::Agent,
        crate::agent_runs::all_orchestration_registered_controllers(),
    );
    // SaaS operator plane: provision and inspect user profiles.
    push_entries(
        &mut controllers,
        DomainGroup::Operator,
        crate::profiles::schemas::all_profiles_controller_entries(),
    );
    controllers
}

/// Aggregates controllers that are registered for RPC routing but NOT exposed to agents.
///
/// These are write-path or internal-only handlers callable by trusted callers
/// (e.g. the desktop shell) that should not appear in agent tool listings.
fn build_internal_only_controllers() -> Vec<GroupedController> {
    let mut controllers = Vec::new();
    #[cfg(feature = "modules")]
    push(
        &mut controllers,
        DomainGroup::Desktop,
        crate::desktop::control::all_registered_controllers(),
    );
    // MCP write audit list: internal-only so the desktop UI/CLI can inspect
    // local write history without exposing cross-client history as an MCP tool.
    push(
        &mut controllers,
        DomainGroup::Mcp,
        crate::mcp::audit::all_mcp_audit_internal_controllers(),
    );
    // Loadable native modules: list/status and an explicit load. Read-only apart
    // from that load, and it cannot name an artifact — the loadable set is
    // compiled into `modules::registry`, so this namespace can start a module
    // the build already trusts and nothing else.
    #[cfg(feature = "modules")]
    push_entries(
        &mut controllers,
        DomainGroup::Modules,
        crate::modules::schemas::all_controller_entries(),
    );
    controllers
}

/// Returns registered controllers filtered by the ambient [`crate::core::runtime::DomainSet`].
pub fn all_registered_controllers() -> Vec<ControllerEntry> {
    let view = registry_view();
    registered_controllers(&view)
}

fn registered_controllers(view: &RegistryView) -> Vec<ControllerEntry> {
    let found = view
        .iter()
        .filter(|g| visible(g))
        .map(|g| g.controller.clone())
        .collect();
    found
}

/// Returns a vector of all controller schemas, derived from the registered
/// controllers (the single source of truth). Kept identical in content to the
/// registered set — schemas can no longer drift from handlers (Phase 2).
///
/// Ambient-filtered by the active [`crate::core::runtime::DomainSet`] just like
/// [`all_registered_controllers`], so `/schema` omits gated namespaces
/// automatically under `harness()`.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    controller_schemas(&registry_view())
}

fn controller_schemas(view: &RegistryView) -> Vec<ControllerSchema> {
    view.iter()
        .filter(|g| visible(g))
        .map(|g| g.controller.schema.clone())
        .collect()
}

/// Generates a standardized RPC method name from a controller schema.
pub fn rpc_method_name(schema: &ControllerSchema) -> String {
    format!("openhuman.{}_{}", schema.namespace, schema.function)
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
    // The full registry (CLI routing, param validation; the DomainSet gate is at
    // dispatch), minus methods the SaaS user surface hides in this scope.
    let view = registry_view();
    let found = view.iter().find(|g| {
        let s = &g.controller.schema;
        s.namespace == namespace && s.function == function && on_surface(g)
    });
    found.map(|g| g.controller.rpc_method_name())
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
    let view = registry_view();
    let found = view
        .iter()
        .chain(internal_registry().iter())
        .find(|g| g.controller.rpc_method_name() == method && visible(g))
        .map(|g| g.controller.schema.clone());
    found
}

/// Validates that the provided parameters match the requirements of the controller schema.
///
/// This is the *single* pre-dispatch gate for every registered controller. Each
/// path that reaches `try_invoke_registered_rpc` validates first:
///
/// | entry point | validates in |
/// | --- | --- |
/// | HTTP JSON-RPC | `openhuman_rpc::server` (through `core::invoke`) |
/// | dynamic dispatch fallback | `core::dispatch::try_registry_dispatch` |
/// | CLI | `core::cli` |
/// | MCP read and write tools | `crate::mcp::server::tools::params` |
///
/// The one call site that does not validate for itself is
/// `crate::mcp::server::write_dispatch`, whose sole caller
/// (`crate::mcp::server::tools::dispatch`) validates immediately before it.
/// That is not a gap today, but it is the place a refactor could open one.
///
/// # Relationship to handler-side parameter checks (#6073)
///
/// Many handlers re-check required params themselves, in wording of their own
/// (`missing required param: id`, ``missing required `class` ``,
/// `invalid params: …`). For an *absent*, *unknown* or *wrong-typed* param a
/// caller never reaches those: this function refuses first, and its message
/// carries the field's *schema comment* as a suffix —
/// `missing required param 'id': Session ID.` — so the two are not
/// interchangeable strings.
///
/// They are not redundant, though. This function deliberately accepts an
/// explicit JSON `null` for any declared type: the required check tests key
/// *presence*, and `check_type` returns early on null (see
/// `validate_params_null_for_required_is_acceptable`). A dispatched
/// `{"class": null}` therefore passes this gate and runs the handler, whose own
/// refusal is exactly what the caller sees. On that path the handler-side check
/// is the only guard there is.
///
/// Two further reasons they stay:
///
/// - Not every `missing required param` in the tree belongs to a registered
///   controller. Agent *tool* handlers take model-produced arguments and are
///   not dispatched through this function at all, so their checks are
///   load-bearing.
/// - A `RegisteredController`'s handler can be invoked directly, and several
///   `tests/raw_coverage` suites do exactly that, asserting the handler's own
///   refusal.
///
/// The practical consequence is for whoever writes the next test: a
/// *missing-key* case asserted against handler wording is testing the handler,
/// not the dispatch path. To pin what a caller actually observes, go through
/// one of the entry points above — see
/// `tests/raw_coverage/dispatch_param_validation_e2e.rs`.
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
            return Err(crate::core::params::missing_required_param_message(
                input.name,
                input.comment,
            ));
        }
    }

    for key in params.keys() {
        if !schema.inputs.iter().any(|f| f.name == key) {
            return Err(crate::core::params::unknown_param_message(
                key,
                schema.namespace,
                schema.function,
            ));
        }
    }

    // Type-check each present param against its declared `TypeSchema`, so every
    // controller gets uniform validation before dispatch rather than relying on
    // each handler's `serde_json::from_value`. Absent (optional) params are
    // already handled by the required-presence check above.
    for input in &schema.inputs {
        if let Some(value) = params.get(input.name) {
            check_type(value, &input.ty).map_err(|mismatch| {
                let (expected, got) = match mismatch {
                    TypeMismatch::Kind(expected) => {
                        (expected.to_string(), crate::core::params::json_type_name(value).to_string())
                    }
                    TypeMismatch::OutOfRange { min, max, got } => {
                        log::debug!(
                            "[rpc][validate] param '{}' in {}.{} out of range: {got} not in {min}..={max}",
                            input.name,
                            schema.namespace,
                            schema.function,
                        );
                        // Name the limit that was actually crossed.
                        let bound = if got > max {
                            format!("unsigned integer <= {max}")
                        } else {
                            format!("unsigned integer >= {min}")
                        };
                        (bound, got.to_string())
                    }
                };
                format!(
                    "invalid type for param '{}' in {}.{}: expected {}, got {}",
                    input.name, schema.namespace, schema.function, expected, got,
                )
            })?;
        }
    }

    Ok(())
}

/// Why a value failed [`check_type`].
enum TypeMismatch {
    /// The JSON kind is wrong; carries a short description of the required type.
    Kind(&'static str),
    /// An unsigned integer outside a [`TypeSchema::BoundedU64`] range.
    ///
    /// [`TypeSchema::BoundedU64`]: crate::core::TypeSchema::BoundedU64
    OutOfRange { min: u64, max: u64, got: u64 },
}

/// Validate a JSON `value` against a declared [`TypeSchema`].
///
/// Returns `Ok(())` on a match, or a [`TypeMismatch`] describing what was
/// required. Unknown/opaque shapes (`Json`, `Bytes`, `Ref`) accept any value —
/// they are validated by the handler's typed deserialization.
fn check_type(value: &Value, ty: &crate::core::TypeSchema) -> Result<(), TypeMismatch> {
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

        TypeSchema::Bool => kind(value.is_boolean(), "bool"),
        TypeSchema::String => kind(value.is_string(), "string"),
        TypeSchema::I64 => kind(value.is_i64(), "integer"),
        TypeSchema::U64 => kind(value.is_u64(), "unsigned integer"),
        TypeSchema::BoundedU64 { min, max } => match value.as_u64() {
            Some(got) if (*min..=*max).contains(&got) => Ok(()),
            Some(got) => Err(TypeMismatch::OutOfRange {
                min: *min,
                max: *max,
                got,
            }),
            None => Err(TypeMismatch::Kind("unsigned integer")),
        },
        TypeSchema::F64 => {
            // Accept any JSON number (ints are valid floats).
            kind(value.is_number(), "number")
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
            None => Err(TypeMismatch::Kind("array")),
        },

        TypeSchema::Enum { variants } => match value.as_str() {
            Some(s) if variants.contains(&s) => Ok(()),
            Some(_) => Err(TypeMismatch::Kind("one of the allowed enum variants")),
            None => Err(TypeMismatch::Kind("string")),
        },
    }
}

/// `Ok(())` when `matches`, else a [`TypeMismatch::Kind`] naming `expected`.
fn kind(matches: bool, expected: &'static str) -> Result<(), TypeMismatch> {
    matches.then_some(()).ok_or(TypeMismatch::Kind(expected))
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

#[cfg(test)]
#[path = "schema_test_support.rs"]
mod schema_test_support;
#[cfg(test)]
pub(crate) use schema_test_support::*;

#[cfg(test)]
#[path = "all_bound_controller_tests.rs"]
mod bound_controller_tests;

#[cfg(test)]
#[path = "all_schema_contract_tests.rs"]
mod schema_contract_tests;

#[path = "all_invoke.rs"]
mod invoke_registered;
pub use invoke_registered::try_invoke_registered_rpc;
#[cfg(test)]
#[path = "all_tests.rs"]
mod tests;
