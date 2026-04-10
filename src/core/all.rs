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
fn build_registered_controllers() -> Vec<RegisteredController> {
    let mut controllers = Vec::new();
    controllers.extend(crate::openhuman::about_app::all_about_app_registered_controllers());
    controllers.extend(crate::openhuman::app_state::all_app_state_registered_controllers());
    controllers.extend(crate::openhuman::cron::all_cron_registered_controllers());
    controllers.extend(crate::openhuman::agent::all_agent_registered_controllers());
    controllers.extend(crate::openhuman::health::all_health_registered_controllers());
    controllers.extend(crate::openhuman::doctor::all_doctor_registered_controllers());
    controllers.extend(crate::openhuman::encryption::all_encryption_registered_controllers());
    controllers.extend(crate::openhuman::heartbeat::all_heartbeat_registered_controllers());
    controllers.extend(crate::openhuman::cost::all_cost_registered_controllers());
    controllers.extend(crate::openhuman::autocomplete::all_autocomplete_registered_controllers());
    controllers.extend(
        crate::openhuman::channels::providers::web::all_web_channel_registered_controllers(),
    );
    controllers
        .extend(crate::openhuman::channels::controllers::all_channels_registered_controllers());
    controllers.extend(crate::openhuman::config::all_config_registered_controllers());
    controllers.extend(crate::openhuman::credentials::all_credentials_registered_controllers());
    controllers.extend(crate::openhuman::service::all_service_registered_controllers());
    controllers.extend(crate::openhuman::migration::all_migration_registered_controllers());
    controllers.extend(crate::openhuman::local_ai::all_local_ai_registered_controllers());
    controllers.extend(
        crate::openhuman::screen_intelligence::all_screen_intelligence_registered_controllers(),
    );
    controllers.extend(crate::openhuman::skills::all_skills_registered_controllers());
    controllers.extend(crate::openhuman::socket::all_socket_registered_controllers());
    controllers.extend(crate::openhuman::workspace::all_workspace_registered_controllers());
    controllers.extend(crate::openhuman::tools::all_tools_registered_controllers());
    controllers.extend(crate::openhuman::memory::all_memory_registered_controllers());
    controllers.extend(crate::openhuman::referral::all_referral_registered_controllers());
    controllers.extend(crate::openhuman::billing::all_billing_registered_controllers());
    controllers.extend(crate::openhuman::team::all_team_registered_controllers());
    controllers.extend(crate::openhuman::text_input::all_text_input_registered_controllers());
    controllers.extend(crate::openhuman::voice::all_voice_registered_controllers());
    controllers.extend(crate::openhuman::subconscious::all_subconscious_registered_controllers());
    controllers.extend(crate::openhuman::webhooks::all_webhooks_registered_controllers());
    controllers.extend(crate::openhuman::update::all_update_registered_controllers());
    controllers
        .extend(crate::openhuman::tree_summarizer::all_tree_summarizer_registered_controllers());
    controllers
}

/// Aggregates all controller schemas from across the codebase.
fn build_declared_controller_schemas() -> Vec<ControllerSchema> {
    let mut schemas = Vec::new();
    schemas.extend(crate::openhuman::about_app::all_about_app_controller_schemas());
    schemas.extend(crate::openhuman::app_state::all_app_state_controller_schemas());
    schemas.extend(crate::openhuman::cron::all_cron_controller_schemas());
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
    schemas.extend(crate::openhuman::skills::all_skills_controller_schemas());
    schemas.extend(crate::openhuman::socket::all_socket_controller_schemas());
    schemas.extend(crate::openhuman::workspace::all_workspace_controller_schemas());
    schemas.extend(crate::openhuman::tools::all_tools_controller_schemas());
    schemas.extend(crate::openhuman::memory::all_memory_controller_schemas());
    schemas.extend(crate::openhuman::referral::all_referral_controller_schemas());
    schemas.extend(crate::openhuman::billing::all_billing_controller_schemas());
    schemas.extend(crate::openhuman::team::all_team_controller_schemas());
    schemas.extend(crate::openhuman::text_input::all_text_input_controller_schemas());
    schemas.extend(crate::openhuman::voice::all_voice_controller_schemas());
    schemas.extend(crate::openhuman::subconscious::all_subconscious_controller_schemas());
    schemas.extend(crate::openhuman::webhooks::all_webhooks_controller_schemas());
    schemas.extend(crate::openhuman::update::all_update_controller_schemas());
    schemas.extend(crate::openhuman::tree_summarizer::all_tree_summarizer_controller_schemas());
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
        "skills" => Some("Skill registry, runtime lifecycle, setup, tools, and sync."),
        "socket" => Some("Skills runtime socket bridge controls."),
        "memory" => Some("Document storage, vector search, key-value store, and knowledge graph."),
        "referral" => Some("Referral codes, stats, and apply flows via the hosted backend API."),
        "billing" => Some("Subscription plan, payment links, and credit top-up via the backend."),
        "team" => Some("Team member management, invites, and role changes via the backend."),
        "voice" => Some("Speech-to-text and text-to-speech using local models."),
        "subconscious" => Some("Periodic local-model background awareness loop."),
        "text_input" => Some("Read, insert, and preview text in the OS-focused input field."),
        "webhooks" => {
            Some("Webhook tunnel registrations and captured request/response debug logs.")
        }
        "update" => {
            Some("Self-update: check GitHub Releases for newer core binary and stage updates.")
        }
        "tree_summarizer" => {
            Some("Hierarchical time-based summarization tree for background knowledge compression.")
        }
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
}
