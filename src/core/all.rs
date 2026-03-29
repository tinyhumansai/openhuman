use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use serde_json::{Map, Value};

use crate::core::ControllerSchema;

pub type ControllerFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'static>>;
pub type ControllerHandler = fn(Map<String, Value>) -> ControllerFuture;

#[derive(Clone)]
pub struct RegisteredController {
    pub schema: ControllerSchema,
    pub handler: ControllerHandler,
}

impl RegisteredController {
    pub fn rpc_method_name(&self) -> String {
        rpc_method_name(&self.schema)
    }
}

static REGISTRY: OnceLock<Vec<RegisteredController>> = OnceLock::new();

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

fn build_registered_controllers() -> Vec<RegisteredController> {
    let mut controllers = Vec::new();
    controllers.extend(crate::openhuman::cron::all_cron_registered_controllers());
    controllers.extend(crate::openhuman::agent::all_agent_registered_controllers());
    controllers.extend(crate::openhuman::health::all_health_registered_controllers());
    controllers.extend(crate::openhuman::doctor::all_doctor_registered_controllers());
    controllers.extend(crate::openhuman::encryption::all_encryption_registered_controllers());
    controllers.extend(crate::openhuman::heartbeat::all_heartbeat_registered_controllers());
    controllers.extend(crate::openhuman::cost::all_cost_registered_controllers());
    controllers.extend(crate::openhuman::autocomplete::all_autocomplete_registered_controllers());
    controllers.extend(crate::openhuman::config::all_config_registered_controllers());
    controllers.extend(crate::openhuman::credentials::all_credentials_registered_controllers());
    controllers.extend(crate::openhuman::service::all_service_registered_controllers());
    controllers.extend(crate::openhuman::migration::all_migration_registered_controllers());
    controllers.extend(crate::openhuman::local_ai::all_local_ai_registered_controllers());
    controllers.extend(
        crate::openhuman::screen_intelligence::all_screen_intelligence_registered_controllers(),
    );
    controllers.extend(crate::openhuman::skills::all_skills_registered_controllers());
    controllers.extend(crate::openhuman::workspace::all_workspace_registered_controllers());
    controllers.extend(crate::openhuman::tray::all_tray_registered_controllers());
    controllers.extend(crate::openhuman::tools::all_tools_registered_controllers());
    controllers
}

fn build_declared_controller_schemas() -> Vec<ControllerSchema> {
    let mut schemas = Vec::new();
    schemas.extend(crate::openhuman::cron::all_cron_controller_schemas());
    schemas.extend(crate::openhuman::agent::all_agent_controller_schemas());
    schemas.extend(crate::openhuman::health::all_health_controller_schemas());
    schemas.extend(crate::openhuman::doctor::all_doctor_controller_schemas());
    schemas.extend(crate::openhuman::encryption::all_encryption_controller_schemas());
    schemas.extend(crate::openhuman::heartbeat::all_heartbeat_controller_schemas());
    schemas.extend(crate::openhuman::cost::all_cost_controller_schemas());
    schemas.extend(crate::openhuman::autocomplete::all_autocomplete_controller_schemas());
    schemas.extend(crate::openhuman::config::all_config_controller_schemas());
    schemas.extend(crate::openhuman::credentials::all_credentials_controller_schemas());
    schemas.extend(crate::openhuman::service::all_service_controller_schemas());
    schemas.extend(crate::openhuman::migration::all_migration_controller_schemas());
    schemas.extend(crate::openhuman::local_ai::all_local_ai_controller_schemas());
    schemas.extend(
        crate::openhuman::screen_intelligence::all_screen_intelligence_controller_schemas(),
    );
    schemas.extend(crate::openhuman::skills::all_skills_controller_schemas());
    schemas.extend(crate::openhuman::workspace::all_workspace_controller_schemas());
    schemas.extend(crate::openhuman::tray::all_tray_controller_schemas());
    schemas.extend(crate::openhuman::tools::all_tools_controller_schemas());
    schemas
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    registry().to_vec()
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    let _ = registry();
    build_declared_controller_schemas()
}

pub fn rpc_method_name(schema: &ControllerSchema) -> String {
    format!("openhuman.{}_{}", schema.namespace, schema.function)
}

pub fn rpc_method_from_parts(namespace: &str, function: &str) -> Option<String> {
    registry()
        .iter()
        .find(|r| r.schema.namespace == namespace && r.schema.function == function)
        .map(|r| r.rpc_method_name())
}

pub fn schema_for_rpc_method(method: &str) -> Option<ControllerSchema> {
    registry()
        .iter()
        .find(|r| r.rpc_method_name() == method)
        .map(|r| r.schema.clone())
}

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
