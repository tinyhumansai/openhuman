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

/// The global static registry of all controllers, initialized once on first access.
static REGISTRY: OnceLock<Vec<RegisteredController>> = OnceLock::new();

/// Returns a reference to the global controller registry.
///
/// This function initializes the registry if it hasn't been already,
/// performing validation to ensure no duplicates or missing handlers exist.
fn registry() -> &'static [RegisteredController] {
    REGISTRY
        .get_or_init(|| {
            let registered = build_registered_controllers();
            let declared = build_declared_controller_schemas();
            validate_registry(&registered, &declared).unwrap_or_else(|err| {
                panic!("invalid controller registry: {err}");
            });
            registered
        })
        .as_slice()
}

/// Aggregates all controller implementations from across the codebase.
///
/// This function is responsible for collecting every domain-specific controller
/// registered in the system. It is used during the initialization of the
/// global [`REGISTRY`].
///
/// When adding a new domain/namespace, its `all_*_registered_controllers()`
/// function must be called here to make it available via RPC and CLI.
fn build_registered_controllers() -> Vec<RegisteredController> {
    let mut controllers = Vec::new();
    // Application information and capabilities
    controllers.extend(crate::openhuman::about_app::all_about_app_registered_controllers());
    // Core application shell state
    controllers.extend(crate::openhuman::app_state::all_app_state_registered_controllers());
    // Composio integration controllers
    controllers.extend(crate::openhuman::composio::all_composio_registered_controllers());
    // Scheduled job management
    controllers.extend(crate::openhuman::cron::all_cron_registered_controllers());
    // Webview APIs bridge — proxies connector calls (Gmail, …) through
    // a WebSocket to the Tauri shell so curl reaches the live webview.
    controllers.extend(crate::openhuman::webview_apis::all_webview_apis_registered_controllers());
    // Agent definition and prompt inspection
    controllers.extend(crate::openhuman::agent::all_agent_registered_controllers());
    // System and process health monitoring
    controllers.extend(crate::openhuman::health::all_health_registered_controllers());
    // Diagnostic tools
    controllers.extend(crate::openhuman::doctor::all_doctor_registered_controllers());
    // Secret storage and encryption
    controllers.extend(crate::openhuman::encryption::all_encryption_registered_controllers());
    // Background heartbeat loop controls
    controllers.extend(crate::openhuman::heartbeat::all_heartbeat_registered_controllers());
    // Token usage and billing cost tracking
    controllers.extend(crate::openhuman::cost::all_cost_registered_controllers());
    // Inline autocomplete settings
    controllers.extend(crate::openhuman::autocomplete::all_autocomplete_registered_controllers());
    // External messaging channels (Web, Telegram, etc.)
    controllers.extend(
        crate::openhuman::channels::providers::web::all_web_channel_registered_controllers(),
    );
    controllers
        .extend(crate::openhuman::channels::controllers::all_channels_registered_controllers());
    // Persistent configuration management
    controllers.extend(crate::openhuman::config::all_config_registered_controllers());
    // User credentials and session management
    controllers.extend(crate::openhuman::credentials::all_credentials_registered_controllers());
    // Desktop service management
    controllers.extend(crate::openhuman::service::all_service_registered_controllers());
    // Data migration utilities
    controllers.extend(crate::openhuman::migration::all_migration_registered_controllers());
    // Local AI model management and inference
    controllers.extend(crate::openhuman::local_ai::all_local_ai_registered_controllers());
    // Screen capture and UI analysis
    controllers.extend(
        crate::openhuman::screen_intelligence::all_screen_intelligence_registered_controllers(),
    );
    // Bridge to external skill runtimes
    controllers.extend(crate::openhuman::socket::all_socket_registered_controllers());
    // Discovered SKILL.md skills and their bundled resources
    controllers.extend(crate::openhuman::skills::all_skills_registered_controllers());
    // User workspace and file management
    controllers.extend(crate::openhuman::workspace::all_workspace_registered_controllers());
    // Skill tool registry
    controllers.extend(crate::openhuman::tools::all_tools_registered_controllers());
    // Document and knowledge graph storage
    controllers.extend(crate::openhuman::memory::all_memory_registered_controllers());
    // Memory tree ingestion layer (#707 — canonicalised chunks with provenance)
    controllers.extend(crate::openhuman::memory::all_memory_tree_registered_controllers());
    // Memory tree retrieval layer (#710 — LLM-callable read tools over the tree)
    controllers.extend(crate::openhuman::memory::all_retrieval_registered_controllers());
    // Slack → memory-tree ingestion engine (backfill + poll + 6hr bucket flush)
    controllers.extend(crate::openhuman::memory::all_slack_ingestion_registered_controllers());
    // Link shortener for long tracking URLs — saves LLM tokens
    controllers
        .extend(crate::openhuman::redirect_links::all_redirect_links_registered_controllers());
    // Referral and growth tracking
    controllers.extend(crate::openhuman::referral::all_referral_registered_controllers());
    // Billing and subscription management
    controllers.extend(crate::openhuman::billing::all_billing_registered_controllers());
    // Team and role management
    controllers.extend(crate::openhuman::team::all_team_registered_controllers());
    // Local assistive surfaces over third-party provider apps
    controllers.extend(
        crate::openhuman::provider_surfaces::all_provider_surfaces_registered_controllers(),
    );
    // OS-level text input interactions
    controllers.extend(crate::openhuman::text_input::all_text_input_registered_controllers());
    // Voice transcription and synthesis
    controllers.extend(crate::openhuman::voice::all_voice_registered_controllers());
    // Background awareness and autonomous tasks
    controllers.extend(crate::openhuman::subconscious::all_subconscious_registered_controllers());
    // Webhook tunnel management
    controllers.extend(crate::openhuman::webhooks::all_webhooks_registered_controllers());
    // Core binary update management
    controllers.extend(crate::openhuman::update::all_update_registered_controllers());
    // Hierarchical knowledge summarization
    controllers
        .extend(crate::openhuman::tree_summarizer::all_tree_summarizer_registered_controllers());
    // Self-learning and user context enrichment
    controllers.extend(crate::openhuman::learning::all_learning_registered_controllers());
    // Conversation thread and message management
    controllers.extend(crate::openhuman::threads::all_threads_registered_controllers());
    // Embedded webview native notifications
    controllers.extend(
        crate::openhuman::webview_notifications::all_webview_notifications_registered_controllers(),
    );
    // Integration notification ingest, triage, and per-provider settings
    controllers.extend(crate::openhuman::notifications::all_notifications_registered_controllers());
    controllers
}

/// Aggregates all controller schemas from across the codebase.
///
/// Similar to [`build_registered_controllers`], but only collects the metadata
/// (schema) for each controller. This is used for discovery and validation.
fn build_declared_controller_schemas() -> Vec<ControllerSchema> {
    let mut schemas = Vec::new();
    schemas.extend(crate::openhuman::about_app::all_about_app_controller_schemas());
    schemas.extend(crate::openhuman::app_state::all_app_state_controller_schemas());
    schemas.extend(crate::openhuman::composio::all_composio_controller_schemas());
    schemas.extend(crate::openhuman::cron::all_cron_controller_schemas());
    schemas.extend(crate::openhuman::webview_apis::all_webview_apis_controller_schemas());
    schemas.extend(crate::openhuman::agent::all_agent_controller_schemas());
    schemas.extend(crate::openhuman::health::all_health_controller_schemas());
    schemas.extend(crate::openhuman::doctor::all_doctor_controller_schemas());
    schemas.extend(crate::openhuman::encryption::all_encryption_controller_schemas());
    schemas.extend(crate::openhuman::heartbeat::all_heartbeat_controller_schemas());
    schemas.extend(crate::openhuman::cost::all_cost_controller_schemas());
    schemas.extend(crate::openhuman::autocomplete::all_autocomplete_controller_schemas());
    schemas
        .extend(crate::openhuman::channels::providers::web::all_web_channel_controller_schemas());
    schemas.extend(crate::openhuman::channels::controllers::all_channels_controller_schemas());
    schemas.extend(crate::openhuman::config::all_config_controller_schemas());
    schemas.extend(crate::openhuman::credentials::all_credentials_controller_schemas());
    schemas.extend(crate::openhuman::service::all_service_controller_schemas());
    schemas.extend(crate::openhuman::migration::all_migration_controller_schemas());
    schemas.extend(crate::openhuman::local_ai::all_local_ai_controller_schemas());
    schemas.extend(
        crate::openhuman::screen_intelligence::all_screen_intelligence_controller_schemas(),
    );
    schemas.extend(crate::openhuman::socket::all_socket_controller_schemas());
    schemas.extend(crate::openhuman::skills::all_skills_controller_schemas());
    schemas.extend(crate::openhuman::workspace::all_workspace_controller_schemas());
    schemas.extend(crate::openhuman::tools::all_tools_controller_schemas());
    schemas.extend(crate::openhuman::memory::all_memory_controller_schemas());
    schemas.extend(crate::openhuman::memory::all_memory_tree_controller_schemas());
    schemas.extend(crate::openhuman::memory::all_retrieval_controller_schemas());
    schemas.extend(crate::openhuman::memory::all_slack_ingestion_controller_schemas());
    schemas.extend(crate::openhuman::redirect_links::all_redirect_links_controller_schemas());
    schemas.extend(crate::openhuman::referral::all_referral_controller_schemas());
    schemas.extend(crate::openhuman::billing::all_billing_controller_schemas());
    schemas.extend(crate::openhuman::team::all_team_controller_schemas());
    schemas.extend(crate::openhuman::provider_surfaces::all_provider_surfaces_controller_schemas());
    schemas.extend(crate::openhuman::text_input::all_text_input_controller_schemas());
    schemas.extend(crate::openhuman::voice::all_voice_controller_schemas());
    schemas.extend(crate::openhuman::subconscious::all_subconscious_controller_schemas());
    schemas.extend(crate::openhuman::webhooks::all_webhooks_controller_schemas());
    schemas.extend(crate::openhuman::update::all_update_controller_schemas());
    schemas.extend(crate::openhuman::tree_summarizer::all_tree_summarizer_controller_schemas());
    schemas.extend(crate::openhuman::learning::all_learning_controller_schemas());
    // Conversation thread and message management
    schemas.extend(crate::openhuman::threads::all_threads_controller_schemas());
    // Embedded webview native notifications
    schemas.extend(
        crate::openhuman::webview_notifications::all_webview_notifications_controller_schemas(),
    );
    // Integration notification ingest, triage, and per-provider settings
    schemas.extend(crate::openhuman::notifications::all_notifications_controller_schemas());
    schemas
}

/// Returns a vector of all currently registered controllers.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    registry().to_vec()
}

/// Returns a vector of all currently declared controller schemas.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    let _ = registry();
    build_declared_controller_schemas()
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
        "app_state" => Some("Expose core-owned app shell state for frontend polling."),
        "auth" => Some("Manage app session and provider credentials."),
        "autocomplete" => Some("Inline autocomplete engine controls and style settings."),
        "channels" => Some("Channel definitions, connections, and lifecycle management."),
        "composio" => Some(
            "Composio OAuth integrations proxied via the backend — toolkits, connections, tools, and actions."
        ),
        "config" => Some("Read and update persisted runtime configuration."),
        "cron" => Some("Manage scheduled jobs and run history."),
        "decrypt" => Some("Decrypt secure values managed by secret storage."),
        "doctor" => Some("Run diagnostics for workspace and runtime health."),
        "encrypt" => Some("Encrypt secure values managed by secret storage."),
        "health" => Some("Process and component health snapshots."),
        "local_ai" => Some("Local AI chat, inference, downloads, and media operations."),
        "migrate" => Some("Data migration utilities."),
        "screen_intelligence" => Some("Screen capture, permissions, and accessibility automation."),
        "service" => Some("Desktop service lifecycle management."),
        "skills" => Some("Discovered SKILL.md skills and their bundled resources."),
        "socket" => Some("Skills runtime socket bridge controls."),
        "memory" => Some("Document storage, vector search, key-value store, and knowledge graph."),
        "memory_tree" => Some(
            "Canonical chunk ingestion, provenance capture, and chunk retrieval for source-grounded memory.",
        ),
        "redirect_links" => Some(
            "Shorten long tracking URLs to `openhuman://link/<id>` placeholders (SQLite-backed) to save tokens in prompts, with round-trip rewrite helpers.",
        ),
        "referral" => Some("Referral codes, stats, and apply flows via the hosted backend API."),
        "billing" => Some("Subscription plan, payment links, and credit top-up via the backend."),
        "team" => Some("Team member management, invites, and role changes via the backend."),
        "provider_surfaces" => Some(
            "Local-first assistive surfaces for provider events, respond queues, and drafts.",
        ),
        "voice" => Some("Speech-to-text and text-to-speech using local models."),
        "subconscious" => Some("Periodic local-model background awareness loop."),
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
        "notification" => Some(
            "Integration notification ingest, triage scoring, listing, read-state, \
             and per-provider routing settings.",
        ),
        _ => None,
    }
}

/// Looks up an RPC method name based on namespace and function.
pub fn rpc_method_from_parts(namespace: &str, function: &str) -> Option<String> {
    registry()
        .iter()
        .find(|r| r.schema.namespace == namespace && r.schema.function == function)
        .map(|r| r.rpc_method_name())
}

/// Retrieves the schema for a specific RPC method.
pub fn schema_for_rpc_method(method: &str) -> Option<ControllerSchema> {
    registry()
        .iter()
        .find(|r| r.rpc_method_name() == method)
        .map(|r| r.schema.clone())
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

    Ok(())
}

/// Attempts to invoke a registered RPC method by name.
///
/// Returns `None` if the method is not found in the registry.
pub async fn try_invoke_registered_rpc(
    method: &str,
    params: Map<String, Value>,
) -> Option<Result<Value, String>> {
    for controller in registry() {
        if controller.rpc_method_name() == method {
            return Some((controller.handler)(params).await);
        }
    }
    None
}

/// Validates the consistency of the controller registry.
///
/// Ensures that:
/// - There are no duplicate controllers or RPC methods.
/// - Every declared schema has a registered handler.
/// - Every registered handler has a declared schema.
/// - Namespaces and functions are not empty.
/// - Required input names are unique within a controller.
fn validate_registry(
    registered: &[RegisteredController],
    declared: &[ControllerSchema],
) -> Result<(), String> {
    use std::collections::{BTreeMap, BTreeSet};

    let mut errors: Vec<String> = Vec::new();
    let mut declared_keys = BTreeSet::new();
    let mut declared_rpc_methods = BTreeSet::new();
    let mut registered_keys = BTreeSet::new();
    let mut registered_rpc_methods = BTreeSet::new();

    for schema in declared {
        let key = format!("{}.{}", schema.namespace, schema.function);
        if !declared_keys.insert(key.clone()) {
            errors.push(format!("duplicate declared controller `{key}`"));
        }

        let rpc_method = rpc_method_name(schema);
        if !declared_rpc_methods.insert(rpc_method.clone()) {
            errors.push(format!("duplicate declared rpc method `{rpc_method}`"));
        }

        if schema.namespace.trim().is_empty() {
            errors.push(format!(
                "invalid declared controller `{key}`: namespace must not be empty"
            ));
        }
        if schema.function.trim().is_empty() {
            errors.push(format!(
                "invalid declared controller `{key}`: function must not be empty"
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

    for controller in registered {
        let key = format!(
            "{}.{}",
            controller.schema.namespace, controller.schema.function
        );
        if !registered_keys.insert(key.clone()) {
            errors.push(format!("duplicate registered controller `{key}`"));
        }

        let rpc_method = controller.rpc_method_name();
        if !registered_rpc_methods.insert(rpc_method.clone()) {
            errors.push(format!("duplicate registered rpc method `{rpc_method}`"));
        }
    }

    for key in declared_keys.difference(&registered_keys) {
        errors.push(format!(
            "declared controller `{key}` has no registered handler"
        ));
    }
    for key in registered_keys.difference(&declared_keys) {
        errors.push(format!(
            "registered controller `{key}` has no declared schema"
        ));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(test)]
#[path = "all_tests.rs"]
mod tests;
