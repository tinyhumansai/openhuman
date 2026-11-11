use super::*;
use crate::core::{FieldSchema, TypeSchema};

/// Independently authored wire fixtures checked against an actual bound
/// registry entry. The runner validates examples with the production schema
/// and decoder, but never invokes its gate or business handler.
#[derive(Clone)]
pub(crate) struct ControllerParamContract {
    pub method: &'static str,
    pub required: &'static [&'static str],
    pub optional: &'static [&'static str],
    pub examples: Vec<Value>,
}

/// Select controllers independently of the fixture table, so a missing fixture
/// cannot silently remove its own controller from the check.
pub(crate) fn verify_parameter_contracts(
    controllers: &[ControllerEntry],
    contracts: &[ControllerParamContract],
) -> Result<(), String> {
    use std::collections::BTreeMap;

    let mut by_method = BTreeMap::new();
    for contract in contracts {
        if by_method.insert(contract.method, contract).is_some() {
            return Err(format!("duplicate contract `{}`", contract.method));
        }
    }
    let mut registered = BTreeMap::new();
    for controller in controllers {
        let method = controller.schema.method_name();
        if registered.insert(method.clone(), controller).is_some() {
            return Err(format!("duplicate controller `{method}`"));
        }
        if !by_method.contains_key(method.as_str()) {
            return Err(format!("missing contract `{method}`"));
        }
    }
    for method in by_method.keys() {
        if !registered.contains_key(*method) {
            return Err(format!("extra contract `{method}`"));
        }
    }
    if registered.is_empty() {
        return Err("empty controller contract inventory".into());
    }
    for (method, controller) in registered {
        let contract = by_method[method.as_str()];
        if !controller.is_bound() {
            return Err(format!("{method} registry entry is legacy, not bound"));
        }
        verify_parameter_schema(&controller.schema, contract)?;
        for (index, example) in contract.examples.iter().enumerate() {
            let params = example
                .as_object()
                .expect("schema verifier rejects non-object examples")
                .clone();
            controller.decode_params(params).map_err(|error| {
                format!("{method} bound decoder rejects example {index}: {error}")
            })?;
        }
    }
    Ok(())
}

pub(crate) fn verify_parameter_schema(
    schema: &ControllerSchema,
    contract: &ControllerParamContract,
) -> Result<(), String> {
    if schema.method_name() != contract.method {
        return Err(format!(
            "{} contract method identity differs",
            contract.method
        ));
    }
    if schema.description.trim().is_empty() {
        return Err(format!("{} has no description", contract.method));
    }
    required_input_contract(schema, contract.required)?;
    optional_input_contract(schema, contract.optional)?;
    if contract.examples.is_empty() {
        return Err(format!("{} has no examples", contract.method));
    }
    for (index, example) in contract.examples.iter().enumerate() {
        let params = example
            .as_object()
            .ok_or_else(|| format!("{} example {index} is not an object", contract.method))?;
        validate_params(schema, params).map_err(|error| {
            format!(
                "{} schema rejects example {index}: {error}",
                contract.method
            )
        })?;
    }
    Ok(())
}

pub(crate) fn verify_wire_fields(
    actual: &[FieldSchema],
    expected: &[(&'static str, TypeSchema, bool)],
) -> Result<(), String> {
    if actual.len() != expected.len() {
        return Err(format!(
            "wire field count mismatch: expected {}, got {}",
            expected.len(),
            actual.len()
        ));
    }
    for (index, (field, (name, ty, required))) in actual.iter().zip(expected).enumerate() {
        if field.name != *name {
            return Err(format!(
                "wire field {index} name mismatch: expected `{name}`, got `{}`",
                field.name
            ));
        }
        if field.ty != *ty {
            return Err(format!(
                "wire field `{name}` type mismatch: expected {ty:?}, got {:?}",
                field.ty
            ));
        }
        if field.required != *required {
            return Err(format!(
                "wire field `{name}` requiredness mismatch: expected {required}, got {}",
                field.required
            ));
        }
    }
    Ok(())
}

pub(crate) fn optional_input_contract(
    schema: &ControllerSchema,
    optional: &[&str],
) -> Result<(), String> {
    let actual: Vec<_> = schema
        .inputs
        .iter()
        .filter(|field| !field.required)
        .map(|field| field.name)
        .collect();
    if actual == optional {
        Ok(())
    } else {
        Err(format!(
            "{}.{} optional inputs mismatch: expected {optional:?}, got {actual:?}",
            schema.namespace, schema.function
        ))
    }
}

/// Owner-authored namespace literals must cover the complete family, including
/// methods without their own field contract; parity alone is self-derived.
pub(crate) fn expect_schema_namespace(schemas: &[ControllerSchema], expected: &str) {
    for schema in schemas {
        assert_eq!(
            schema.namespace, expected,
            "{} wire namespace",
            schema.function
        );
    }
}

pub(crate) fn expect_schema_identity(schema: &ControllerSchema, namespace: &str, function: &str) {
    assert_eq!(schema.namespace, namespace);
    assert_eq!(schema.function, function);
}

pub(crate) fn expect_schema_fields(schema: &ControllerSchema, fields: &[(&str, Option<bool>)]) {
    for (name, required) in fields {
        let field = schema
            .inputs
            .iter()
            .find(|field| field.name == *name)
            .unwrap_or_else(|| {
                panic!(
                    "{}.{} is missing input `{name}`",
                    schema.namespace, schema.function
                )
            });
        if let Some(required) = required {
            assert_eq!(
                field.required, *required,
                "{}.{} input `{name}`",
                schema.namespace, schema.function
            );
        }
    }
}

pub(crate) fn expect_schema_required_inputs(schema: &ControllerSchema, required: &[&str]) {
    assert_eq!(
        required_input_contract(schema, required),
        Ok(()),
        "{}.{} required inputs",
        schema.namespace,
        schema.function
    );
}

pub(crate) fn required_input_contract(
    schema: &ControllerSchema,
    required: &[&str],
) -> Result<(), String> {
    let actual: Vec<_> = schema
        .inputs
        .iter()
        .filter(|field| field.required)
        .map(|field| field.name)
        .collect();
    if actual == required {
        Ok(())
    } else {
        Err(format!(
            "{}.{} required inputs mismatch: expected {required:?}, got {actual:?}",
            schema.namespace, schema.function
        ))
    }
}

pub(crate) fn required_fields_contract(
    schema: &ControllerSchema,
    required: &[&str],
) -> Result<(), String> {
    for name in required {
        let Some(field) = schema.inputs.iter().find(|field| field.name == *name) else {
            return Err(format!(
                "{}.{} is missing required input `{name}`",
                schema.namespace, schema.function
            ));
        };
        if !field.required {
            return Err(format!(
                "{}.{} input `{name}` is not required",
                schema.namespace, schema.function
            ));
        }
    }
    Ok(())
}

pub(crate) fn expect_schema_input_order(schema: &ControllerSchema, inputs: &[&str]) {
    let actual: Vec<_> = schema.inputs.iter().map(|field| field.name).collect();
    assert_eq!(
        actual, inputs,
        "{}.{} input order",
        schema.namespace, schema.function
    );
}

pub(crate) fn expect_schema_output_names(schema: &ControllerSchema, outputs: &[&str]) {
    let actual: Vec<_> = schema.outputs.iter().map(|field| field.name).collect();
    assert_eq!(
        actual, outputs,
        "{}.{} output names",
        schema.namespace, schema.function
    );
}

pub(crate) fn expect_schema_output_fields(schema: &ControllerSchema, outputs: &[&str]) {
    for name in outputs {
        assert!(
            schema.outputs.iter().any(|field| field.name == *name),
            "{}.{} is missing output `{name}`",
            schema.namespace,
            schema.function
        );
    }
}

/// Shared test helper: assert a domain's controller-schema list and
/// registered-controller list stay in lockstep, and that a known function is
/// present. Replaces the brittle `assert_eq!(schemas().len(), N)` magic-number
/// pattern repeated across ~15 domains (plan.md §3/§6) — a legitimate new
/// controller no longer breaks the count, but a schema/handler desync or a
/// dropped op still fails.
pub(crate) fn assert_schema_controller_parity(
    schemas: &[ControllerSchema],
    controllers: &[RegisteredController],
    known_function: &str,
) {
    assert_eq!(schemas.len(), controllers.len(), "schema/controller count");
    for (schema, controller) in schemas.iter().zip(controllers) {
        assert_eq!(
            schema.method_name(),
            controller.schema.method_name(),
            "schema/controller registration order diverged"
        );
    }
    assert!(
        schemas.iter().any(|s| s.function == known_function),
        "expected a `{known_function}` controller schema to be registered",
    );
}

#[cfg(test)]
#[path = "schema_test_support_tests.rs"]
mod tests;
