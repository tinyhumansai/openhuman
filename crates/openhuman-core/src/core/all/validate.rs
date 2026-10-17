//! Registry consistency validation.

use super::GroupedController;

/// Validates the consistency of the controller registry.
///
/// The registry is the single source of truth: each [`RegisteredController`]
/// carries its own schema, and [`all_controller_schemas`] derives the public
/// list. No separate declaration can drift from handlers (Phase 2).
///
/// Ensures that:
/// - There are no duplicate controllers or RPC methods.
/// - Namespaces and functions are not empty.
/// - Required input names are unique within a controller.
pub(super) fn validate_registry(registered: &[GroupedController]) -> Result<(), String> {
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
