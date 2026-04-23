//! Shared core-level schemas and contracts used across adapters (RPC, CLI, etc.).
//!
//! This module defines the foundational types for OpenHuman's controller system,
//! which provides a transport-agnostic way to define and invoke domain logic.
//! It also exports submodules for CLI handling, event bus, and RPC server.

use serde::Serialize;

pub mod agent_cli;
pub mod all;
pub mod autocomplete_cli_adapter;
pub mod cli;
pub mod dispatch;
pub mod event_bus;
pub mod jsonrpc;
pub mod logging;
pub mod memory_cli;
pub mod rpc_log;
pub mod shutdown;
pub mod socketio;
pub mod types;

/// Canonical function contract for domain controllers.
///
/// This shape is transport-agnostic and can be consumed by RPC and CLI layers
/// in different ways. It defines the identity, purpose, and I/O signature
/// of a specific piece of domain logic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ControllerSchema {
    /// Domain/group identifier, e.g. `memory`, `config`, `credentials`.
    /// This forms the first part of the RPC method name.
    pub namespace: &'static str,
    /// Function identifier inside namespace, e.g. `doc_put`.
    /// This forms the second part of the RPC method name.
    pub function: &'static str,
    /// One-line human-readable purpose, used for CLI help and API documentation.
    pub description: &'static str,
    /// Ordered input parameters accepted by the controller function.
    /// Each input is a field with a name, type, and description.
    pub inputs: Vec<FieldSchema>,
    /// Ordered output fields returned by the controller function.
    /// This defines the structure of the successful response.
    pub outputs: Vec<FieldSchema>,
}

impl ControllerSchema {
    /// Canonical dotted name for routing, e.g. `memory.doc_put`.
    /// This is used internally to identify the controller.
    pub fn method_name(&self) -> String {
        format!("{}.{}", self.namespace, self.function)
    }
}

/// Schema for one input/output field.
///
/// Defines the properties of a single parameter or return value,
/// enabling validation and documentation generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldSchema {
    /// Field name. Used as the key in JSON objects or as a CLI flag.
    pub name: &'static str,
    /// Field type, defining the expected data shape and enabling validation.
    pub ty: TypeSchema,
    /// Human-readable description for docs/help. Should explain what the field is for.
    pub comment: &'static str,
    /// Requiredness for adapters:
    /// - input: if true, the argument/flag MUST be provided.
    /// - output: if true, the field is guaranteed to be present in the response.
    pub required: bool,
}

/// Type-system shape used by controller input/output schema fields.
///
/// This enum represents the set of supported types that can be passed
/// across the controller boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum TypeSchema {
    /// A boolean value (true/false).
    Bool,
    /// A 64-bit signed integer.
    I64,
    /// A 64-bit unsigned integer.
    U64,
    /// A 64-bit floating point number.
    F64,
    /// A UTF-8 encoded string.
    String,
    /// A generic JSON value (serde_json::Value).
    Json,
    /// Raw binary data.
    Bytes,
    /// An ordered list of values of a specific type.
    Array(Box<TypeSchema>),
    /// String-keyed map/object with homogeneous values.
    Map(Box<TypeSchema>),
    /// An optional value that may be null or a value of the inner type.
    Option(Box<TypeSchema>),
    /// A string that must match one of the predefined variants.
    Enum {
        /// The list of allowed string variants.
        variants: Vec<&'static str>,
    },
    /// A nested object with its own set of fields.
    Object {
        /// The fields defining the object's structure.
        fields: Vec<FieldSchema>,
    },
    /// Reference to a named shared/domain type defined elsewhere.
    Ref(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(namespace: &'static str, function: &'static str) -> ControllerSchema {
        ControllerSchema {
            namespace,
            function,
            description: "",
            inputs: vec![],
            outputs: vec![],
        }
    }

    #[test]
    fn method_name_joins_namespace_and_function_with_dot() {
        let s = mk("memory", "doc_put");
        assert_eq!(s.method_name(), "memory.doc_put");
    }

    #[test]
    fn method_name_is_not_an_rpc_method_name() {
        // The dotted controller key and the `openhuman.<ns>_<fn>` RPC method
        // name are intentionally different — guard against drift.
        let s = mk("memory", "doc_put");
        assert_eq!(s.method_name(), "memory.doc_put");
        assert_eq!(
            crate::core::all::rpc_method_name(&s),
            "openhuman.memory_doc_put"
        );
    }

    #[test]
    fn method_name_preserves_underscores_in_function() {
        let s = mk("team", "change_member_role");
        assert_eq!(s.method_name(), "team.change_member_role");
    }

    #[test]
    fn controller_schema_equality_considers_all_fields() {
        let a = ControllerSchema {
            namespace: "a",
            function: "b",
            description: "x",
            inputs: vec![],
            outputs: vec![],
        };
        let b = ControllerSchema {
            namespace: "a",
            function: "b",
            description: "x",
            inputs: vec![],
            outputs: vec![],
        };
        let c = ControllerSchema {
            namespace: "a",
            function: "b",
            description: "different",
            inputs: vec![],
            outputs: vec![],
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn type_schema_nesting_is_equality_comparable() {
        let a = TypeSchema::Array(Box::new(TypeSchema::Option(Box::new(TypeSchema::String))));
        let b = TypeSchema::Array(Box::new(TypeSchema::Option(Box::new(TypeSchema::String))));
        let c = TypeSchema::Array(Box::new(TypeSchema::Option(Box::new(TypeSchema::I64))));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn field_schema_required_flag_changes_equality() {
        let a = FieldSchema {
            name: "x",
            ty: TypeSchema::Bool,
            comment: "",
            required: true,
        };
        let b = FieldSchema {
            name: "x",
            ty: TypeSchema::Bool,
            comment: "",
            required: false,
        };
        assert_ne!(a, b);
    }

    #[test]
    fn controller_schema_serializes_to_json() {
        // Schema must be JSON-serializable: the /schema endpoint depends on it.
        let s = ControllerSchema {
            namespace: "health",
            function: "snapshot",
            description: "d",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::U64,
                comment: "cap",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "",
                required: true,
            }],
        };
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["namespace"], "health");
        assert_eq!(json["function"], "snapshot");
        assert_eq!(json["inputs"][0]["name"], "limit");
        assert_eq!(json["outputs"][0]["required"], true);
    }
}
