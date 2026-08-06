//! MCP HTTP transport primitives — **always compiled**.
//!
//! [`McpHttpClient`] speaks Streamable HTTP + OAuth + SSE per the MCP spec and
//! is a reusable building block for any module that needs to *talk to* a
//! remote MCP server.
//!
//! # Why this is not gated
//!
//! This module is a mis-housed shared utility: two always-compiled consumers
//! reach it and neither is part of the MCP subsystem.
//!
//! * The bespoke `gitbooks` docs tool
//!   ([`crate::openhuman::tools::r#impl::network`]) dials [`McpHttpClient`] and
//!   [`redact_endpoint`] directly — GitBook is modelled as a legacy MCP
//!   server. Stubbing it would break a docs tool users reach in slim builds.
//! * `core::observability` names [`McpUnauthorizedError`] in its
//!   always-compiled `McpServerNeedsAuth` classifier coupling test. Keeping
//!   this compiled keeps that test free of `#[cfg]` and of wording drift.
//!
//! The gate follows the real dependency graph, not the directory name.

mod client;

// Shared JSON-RPC result renderer. Before the family split this was reached as
// `super::client::render_tool_result` from the sibling stdio transport; that
// sibling now lives in the gated `config_servers`, so the re-export carries the
// same gate. The function itself is compiled in both builds.
#[cfg(feature = "mcp")]
pub(crate) use client::render_tool_result;
pub use client::{
    redact_endpoint, AuthorizationServerMetadata, McpAuthChallenge, McpAuthorizationContext,
    McpHttpClient, McpInitializeResult, McpRemoteTool, McpServerToolResult, McpSseEvent,
    McpUnauthorizedError, ProtectedResourceMetadata,
};
