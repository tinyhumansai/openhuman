use std::future::Future;
use std::pin::Pin;

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

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    crate::openhuman::cron::all_cron_registered_controllers()
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    all_registered_controllers()
        .into_iter()
        .map(|r| r.schema)
        .collect()
}

pub fn rpc_method_name(schema: &ControllerSchema) -> String {
    format!("openhuman.{}_{}", schema.namespace, schema.function)
}

pub fn rpc_method_from_parts(namespace: &str, function: &str) -> Option<String> {
    all_registered_controllers()
        .into_iter()
        .find(|r| r.schema.namespace == namespace && r.schema.function == function)
        .map(|r| r.rpc_method_name())
}

pub fn schema_for_rpc_method(method: &str) -> Option<ControllerSchema> {
    all_registered_controllers()
        .into_iter()
        .find(|r| r.rpc_method_name() == method)
        .map(|r| r.schema)
}

pub fn validate_params(schema: &ControllerSchema, params: &Map<String, Value>) -> Result<(), String> {
    for input in &schema.inputs {
        if input.required && !params.contains_key(input.name) {
            return Err(format!("missing required param '{}': {}", input.name, input.comment));
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
    for controller in all_registered_controllers() {
        if controller.rpc_method_name() == method {
            return Some((controller.handler)(params).await);
        }
    }
    None
}
