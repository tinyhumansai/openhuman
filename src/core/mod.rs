//! Shared core-level schemas and contracts used across adapters (RPC, CLI, etc.).
use serde::Serialize;

pub mod all;
pub mod cli;
pub mod dispatch;
pub mod jsonrpc;
pub mod rpc_log;
pub mod socketio;
pub mod types;

/// Canonical function contract for domain controllers.
///
/// This shape is transport-agnostic and can be consumed by RPC and CLI layers
/// in different ways.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ControllerSchema {
    /// Domain/group identifier, e.g. `memory`, `config`, `credentials`.
    pub namespace: &'static str,
    /// Function identifier inside namespace, e.g. `doc_put`.
    pub function: &'static str,
    /// One-line human-readable purpose.
    pub description: &'static str,
    /// Ordered input parameters accepted by the controller function.
    pub inputs: Vec<FieldSchema>,
    /// Ordered output fields returned by the controller function.
    pub outputs: Vec<FieldSchema>,
}

impl ControllerSchema {
    /// Canonical dotted name for routing, e.g. `memory.doc_put`.
    pub fn method_name(&self) -> String {
        format!("{}.{}", self.namespace, self.function)
    }
}

/// Schema for one input/output field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldSchema {
    /// Field name.
    pub name: &'static str,
    /// Field type.
    pub ty: TypeSchema,
    /// Human-readable description for docs/help.
    pub comment: &'static str,
    /// Requiredness for adapters:
    /// - input: required argument/flag
    /// - output: always-present field when true
    pub required: bool,
}

/// Type-system shape used by controller input/output schema fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum TypeSchema {
    Bool,
    I64,
    U64,
    F64,
    String,
    Json,
    Bytes,
    Array(Box<TypeSchema>),
    /// String-keyed map/object with homogeneous values.
    Map(Box<TypeSchema>),
    Option(Box<TypeSchema>),
    Enum {
        variants: Vec<&'static str>,
    },
    Object {
        fields: Vec<FieldSchema>,
    },
    /// Reference to a named shared/domain type.
    Ref(&'static str),
}
