//! Controller schemas and handler dispatch for the MCP clients domain.
//!
//! Every `schemas(function)` match arm defines the RPC method's input/output
//! shape. Every `handle_*` function deserialises params and delegates to
//! `ops.rs`.

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
include!("schemas_part_01.rs");
include!("schemas_part_02.rs");
