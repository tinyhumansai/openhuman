//! Shared MCP client + registry for remote MCP servers exposed to agents.
//!
//! Supports Streamable HTTP and stdio transports. HTTP transport carries
//! session + auth lifecycle; stdio launches a subprocess and exchanges
//! newline-delimited JSON-RPC messages over stdin/stdout per the MCP spec.

mod client;
mod registry;
mod stdio;

pub use client::{
    redact_endpoint, AuthorizationServerMetadata, McpAuthChallenge, McpAuthorizationContext,
    McpHttpClient, McpInitializeResult, McpRemoteTool, McpServerToolResult, McpSseEvent,
    ProtectedResourceMetadata,
};
pub use registry::{McpRegistrySource, McpServerDefinition, McpServerRegistry, McpTransportClient};
pub use stdio::McpStdioClient;
