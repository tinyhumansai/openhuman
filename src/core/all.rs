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
mod tests {
    use serde_json::Map;

    use super::*;
    use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

    fn schema(
        namespace: &'static str,
        function: &'static str,
        inputs: Vec<FieldSchema>,
    ) -> ControllerSchema {
        ControllerSchema {
            namespace,
            function,
            description: "test",
            inputs,
            outputs: vec![],
        }
    }

    fn noop_handler(_params: Map<String, Value>) -> ControllerFuture {
        Box::pin(async { Ok(Value::Null) })
    }

    #[test]
    fn validate_registry_rejects_duplicate_namespace_function() {
        let declared = vec![schema("dup", "fn", vec![]), schema("dup", "fn", vec![])];
        let registered = vec![
            RegisteredController {
                schema: declared[0].clone(),
                handler: noop_handler,
            },
            RegisteredController {
                schema: declared[1].clone(),
                handler: noop_handler,
            },
        ];

        let err = validate_registry(&registered, &declared).expect_err("expected duplicate error");
        assert!(err.contains("duplicate declared controller `dup.fn`"));
    }

    #[test]
    fn validate_registry_rejects_duplicate_required_inputs() {
        let declared = vec![schema(
            "doctor",
            "models",
            vec![
                FieldSchema {
                    name: "use_cache",
                    ty: TypeSchema::Bool,
                    comment: "x",
                    required: true,
                },
                FieldSchema {
                    name: "use_cache",
                    ty: TypeSchema::Bool,
                    comment: "x",
                    required: true,
                },
            ],
        )];
        let registered = vec![RegisteredController {
            schema: declared[0].clone(),
            handler: noop_handler,
        }];

        let err = validate_registry(&registered, &declared).expect_err("expected duplicate input");
        assert!(err.contains("duplicate required input `use_cache` in `doctor.models`"));
    }

    #[test]
    fn validate_registry_accepts_valid_registry() {
        let declared = vec![
            schema("ns1", "fn1", vec![]),
            schema("ns1", "fn2", vec![]),
            schema("ns2", "fn1", vec![]),
        ];
        let registered = declared
            .iter()
            .map(|s| RegisteredController {
                schema: s.clone(),
                handler: noop_handler,
            })
            .collect::<Vec<_>>();
        assert!(validate_registry(&registered, &declared).is_ok());
    }

    #[test]
    fn rpc_method_name_formats_correctly() {
        let s = schema("memory", "doc_put", vec![]);
        assert_eq!(rpc_method_name(&s), "openhuman.memory_doc_put");
    }

    #[test]
    fn registered_controller_rpc_method_name() {
        let s = schema("billing", "get_balance", vec![]);
        let rc = RegisteredController {
            schema: s,
            handler: noop_handler,
        };
        assert_eq!(rc.rpc_method_name(), "openhuman.billing_get_balance");
    }

    #[test]
    fn namespace_description_known_namespaces() {
        assert!(namespace_description("memory").is_some());
        assert!(namespace_description("memory_tree").is_some());
        assert!(namespace_description("redirect_links").is_some());
        assert!(namespace_description("billing").is_some());
        assert!(namespace_description("config").is_some());
        assert!(namespace_description("health").is_some());
        assert!(namespace_description("voice").is_some());
        assert!(namespace_description("webhooks").is_some());
        assert!(namespace_description("notification").is_some());
    }

    #[test]
    fn namespace_description_unknown_returns_none() {
        assert!(namespace_description("nonexistent_xyz").is_none());
    }

    #[test]
    fn validate_params_accepts_valid_params() {
        let s = schema(
            "test",
            "fn",
            vec![FieldSchema {
                name: "key",
                ty: TypeSchema::String,
                comment: "a key",
                required: true,
            }],
        );
        let mut params = Map::new();
        params.insert("key".into(), Value::String("value".into()));
        assert!(validate_params(&s, &params).is_ok());
    }

    #[test]
    fn validate_params_rejects_missing_required() {
        let s = schema(
            "test",
            "fn",
            vec![FieldSchema {
                name: "key",
                ty: TypeSchema::String,
                comment: "a key",
                required: true,
            }],
        );
        let params = Map::new();
        let err = validate_params(&s, &params).unwrap_err();
        assert!(err.contains("missing required param 'key'"));
    }

    #[test]
    fn validate_params_rejects_unknown_param() {
        let s = schema("test", "fn", vec![]);
        let mut params = Map::new();
        params.insert("unknown".into(), Value::Null);
        let err = validate_params(&s, &params).unwrap_err();
        assert!(err.contains("unknown param 'unknown'"));
    }

    #[test]
    fn validate_params_accepts_empty_for_no_required() {
        let s = schema("test", "fn", vec![]);
        assert!(validate_params(&s, &Map::new()).is_ok());
    }

    #[test]
    fn all_registered_controllers_is_nonempty() {
        let controllers = all_registered_controllers();
        assert!(
            controllers.len() > 50,
            "expected many controllers, got {}",
            controllers.len()
        );
    }

    #[test]
    fn all_controller_schemas_matches_registered_count() {
        let schemas = all_controller_schemas();
        let controllers = all_registered_controllers();
        assert_eq!(schemas.len(), controllers.len());
    }

    #[test]
    fn schema_for_rpc_method_finds_known_method() {
        let schema = schema_for_rpc_method("openhuman.health_snapshot");
        assert!(schema.is_some(), "health.snapshot should be findable");
        let s = schema.unwrap();
        assert_eq!(s.namespace, "health");
        assert_eq!(s.function, "snapshot");
    }

    #[test]
    fn schema_for_rpc_method_returns_none_for_unknown() {
        assert!(schema_for_rpc_method("openhuman.nonexistent_method_xyz").is_none());
    }

    #[test]
    fn rpc_method_from_parts_finds_known() {
        let method = rpc_method_from_parts("health", "snapshot");
        assert_eq!(method.as_deref(), Some("openhuman.health_snapshot"));
    }

    #[test]
    fn rpc_method_from_parts_returns_none_for_unknown() {
        assert!(rpc_method_from_parts("fake", "method").is_none());
    }

    #[test]
    fn no_duplicate_rpc_methods_in_registry() {
        let controllers = all_registered_controllers();
        let mut methods: Vec<String> = controllers.iter().map(|c| c.rpc_method_name()).collect();
        let original_len = methods.len();
        methods.sort();
        methods.dedup();
        assert_eq!(
            methods.len(),
            original_len,
            "duplicate RPC methods found in registry"
        );
    }

    // --- validate_params edge cases -----------------------------------------

    #[test]
    fn validate_params_accepts_missing_optional_param() {
        let s = schema(
            "test",
            "fn",
            vec![FieldSchema {
                name: "filter",
                ty: TypeSchema::String,
                comment: "optional filter",
                required: false,
            }],
        );
        assert!(validate_params(&s, &Map::new()).is_ok());
    }

    #[test]
    fn validate_params_accepts_optional_param_when_present() {
        let s = schema(
            "test",
            "fn",
            vec![FieldSchema {
                name: "filter",
                ty: TypeSchema::String,
                comment: "",
                required: false,
            }],
        );
        let mut p = Map::new();
        p.insert("filter".into(), Value::String("abc".into()));
        assert!(validate_params(&s, &p).is_ok());
    }

    #[test]
    fn validate_params_missing_required_error_includes_comment() {
        // The comment text helps callers (esp. the CLI/UI) understand what
        // the missing field is for — lock this in so error messages don't
        // regress to bare field names.
        let s = schema(
            "memory",
            "doc_put",
            vec![FieldSchema {
                name: "namespace",
                ty: TypeSchema::String,
                comment: "namespace to write into",
                required: true,
            }],
        );
        let err = validate_params(&s, &Map::new()).unwrap_err();
        assert!(err.contains("missing required param 'namespace'"));
        assert!(err.contains("namespace to write into"));
    }

    #[test]
    fn validate_params_unknown_error_includes_namespace_and_function() {
        let s = schema("billing", "top_up", vec![]);
        let mut p = Map::new();
        p.insert("typo".into(), Value::Null);
        let err = validate_params(&s, &p).unwrap_err();
        assert!(err.contains("unknown param 'typo'"));
        assert!(err.contains("billing.top_up"));
    }

    #[test]
    fn validate_params_reports_missing_required_before_unknown() {
        // If a call both omits a required param AND has an unknown one,
        // the missing-required error fires first (it's strictly more
        // actionable for callers).
        let s = schema(
            "test",
            "fn",
            vec![FieldSchema {
                name: "key",
                ty: TypeSchema::String,
                comment: "",
                required: true,
            }],
        );
        let mut p = Map::new();
        p.insert("unknown".into(), Value::Null);
        let err = validate_params(&s, &p).unwrap_err();
        assert!(err.contains("missing required param 'key'"), "got: {err}");
    }

    #[test]
    fn validate_params_null_for_required_is_acceptable() {
        // JSON-RPC semantics: `null` is a valid value for an optional field
        // sent explicitly. For a required field, presence (not value) is
        // what we check — null does satisfy the "key present" check.
        // Handlers enforce stronger type contracts downstream.
        let s = schema(
            "test",
            "fn",
            vec![FieldSchema {
                name: "key",
                ty: TypeSchema::String,
                comment: "",
                required: true,
            }],
        );
        let mut p = Map::new();
        p.insert("key".into(), Value::Null);
        assert!(validate_params(&s, &p).is_ok());
    }

    // --- validate_registry edge cases ---------------------------------------

    #[test]
    fn validate_registry_rejects_empty_namespace() {
        let declared = vec![schema("", "fn", vec![])];
        let registered = vec![RegisteredController {
            schema: declared[0].clone(),
            handler: noop_handler,
        }];
        let err = validate_registry(&registered, &declared).unwrap_err();
        assert!(err.contains("namespace must not be empty"));
    }

    #[test]
    fn validate_registry_rejects_empty_function() {
        let declared = vec![schema("ns", "", vec![])];
        let registered = vec![RegisteredController {
            schema: declared[0].clone(),
            handler: noop_handler,
        }];
        let err = validate_registry(&registered, &declared).unwrap_err();
        assert!(err.contains("function must not be empty"));
    }

    #[test]
    fn validate_registry_rejects_whitespace_only_namespace() {
        // `trim().is_empty()` is the invariant — a namespace of "   " must
        // be rejected to prevent `openhuman.   _fn` nonsense RPC method names.
        let declared = vec![schema("   ", "fn", vec![])];
        let registered = vec![RegisteredController {
            schema: declared[0].clone(),
            handler: noop_handler,
        }];
        let err = validate_registry(&registered, &declared).unwrap_err();
        assert!(err.contains("namespace must not be empty"));
    }

    #[test]
    fn validate_registry_rejects_declared_without_registered() {
        let declared = vec![schema("a", "b", vec![])];
        let registered: Vec<RegisteredController> = vec![];
        let err = validate_registry(&registered, &declared).unwrap_err();
        assert!(err.contains("declared controller `a.b` has no registered handler"));
    }

    #[test]
    fn validate_registry_rejects_registered_without_declared() {
        let declared: Vec<ControllerSchema> = vec![];
        let registered = vec![RegisteredController {
            schema: schema("a", "b", vec![]),
            handler: noop_handler,
        }];
        let err = validate_registry(&registered, &declared).unwrap_err();
        assert!(err.contains("registered controller `a.b` has no declared schema"));
    }

    #[test]
    fn validate_registry_rejects_duplicate_registered_controllers() {
        let s = schema("a", "b", vec![]);
        let declared = vec![s.clone()];
        let registered = vec![
            RegisteredController {
                schema: s.clone(),
                handler: noop_handler,
            },
            RegisteredController {
                schema: s,
                handler: noop_handler,
            },
        ];
        let err = validate_registry(&registered, &declared).unwrap_err();
        assert!(err.contains("duplicate registered controller `a.b`"));
    }

    // --- try_invoke_registered_rpc routing ---------------------------------

    #[tokio::test]
    async fn try_invoke_registered_rpc_returns_none_for_unknown_method() {
        let out =
            try_invoke_registered_rpc("openhuman.not_a_real_method_xyz_123", Map::new()).await;
        assert!(out.is_none(), "unknown methods must return None");
    }

    #[tokio::test]
    async fn try_invoke_registered_rpc_returns_some_for_known_method() {
        // `openhuman.health_snapshot` is registered at startup and takes no
        // required params — it must route and produce Some(_).
        let out = try_invoke_registered_rpc("openhuman.health_snapshot", Map::new()).await;
        assert!(out.is_some(), "known method must route");
    }

    #[test]
    fn rpc_method_name_handles_multi_underscore_function() {
        // Functions often contain underscores — the RPC method name must
        // preserve them verbatim, separated from the namespace with `_`.
        let s = schema("team", "change_member_role", vec![]);
        assert_eq!(rpc_method_name(&s), "openhuman.team_change_member_role");
    }

    #[test]
    fn every_registered_controller_has_matching_declared_schema() {
        // Global invariant: the registry is consistent by construction.
        // This test re-asserts the contract to catch drift.
        use std::collections::BTreeSet;
        let registered: BTreeSet<String> = all_registered_controllers()
            .into_iter()
            .map(|c| format!("{}.{}", c.schema.namespace, c.schema.function))
            .collect();
        let declared: BTreeSet<String> = all_controller_schemas()
            .into_iter()
            .map(|s| format!("{}.{}", s.namespace, s.function))
            .collect();
        assert_eq!(
            registered, declared,
            "registry/schema sets must be identical"
        );
    }
}
