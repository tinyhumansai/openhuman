//! Shared core-level schemas and contracts used across adapters (RPC, CLI, etc.).
//!
//! This module defines the foundational types for OpenHuman's controller system,
//! which provides a transport-agnostic way to define and invoke domain logic.
//! It also exports submodules for CLI handling, event bus, and RPC server.

use serde::Serialize;

pub mod agent_cli;
pub mod all;
pub mod auth;
pub mod bus;
pub mod bus_testing;
pub mod cli;
pub mod dispatch;
pub mod envelope;
pub mod event_bind_tokens;
pub mod events;
// Ungated compile-time marker for the `http-server` gate (#5048) — the desktop
// shell asserts `HTTP_SERVER_COMPILED_IN` so a listener-less core fails the
// build instead of shipping silently (cf. voice #4901).
pub mod http_server_status;
pub mod invoke;
pub mod legacy_aliases;
pub mod log_redaction;
pub mod logging;
pub mod observability;
pub mod outcome;
pub mod params;
pub mod rpc_log;
pub mod runtime;
#[cfg(feature = "crash-reporting")]
pub mod sentry_transport;
pub mod server_launcher;
pub mod session_expiry;
pub mod shutdown;
pub mod structured_error;
pub mod subsystem;
pub mod subsystems_cli;
pub mod types;

pub use outcome::{apply_log_envelope, unwrap_rpc, Outcome};
pub use structured_error::{StructuredRpcError, STRUCTURED_RPC_ERROR_SENTINEL};

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
    /// An unsigned integer confined to `min..=max` (both inclusive).
    ///
    /// Declares the real range of a field backed by a type narrower than
    /// `u64` (`u32`, `u16`, `u8`, `NonZero*`), so `validate_params` refuses an
    /// out-of-range value before dispatch instead of the handler's typed
    /// deserialization failing after it (#6137). A plain `u32` field is
    /// `BoundedU64 { min: 0, max: u32::MAX as u64 }`.
    BoundedU64 {
        /// Smallest accepted value.
        min: u64,
        /// Largest accepted value.
        max: u64,
    },
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
#[path = "core_mod_tests.rs"]
mod tests;
