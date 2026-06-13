//! Third-party MCP-server sync pipelines.
//!
//! Pipelines that pull from MCP (Model Context Protocol) servers the user
//! has connected. One pipeline per server.
//!
//! ## Layer rules
//!
//! - Transport (stdio / SSE / websocket) is owned by `mcp_clients/`; sync
//!   here calls into that surface, never re-implements it.
//! - Data shapes are MCP-generic — the pipeline normalises into raw md
//!   per record so the rest of memory_store doesn't have to know about
//!   MCP at all.
//!
//! ## Status
//!
//! Scaffold only. The existing `mcp_clients/` module already knows how
//! to talk to a server; what's missing is the "drain new records since
//! last cursor and ingest" loop on top.
