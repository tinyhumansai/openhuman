//! OpenHuman — a lightweight agent runtime for human-AI collaboration.
//!
//! The `openhuman` module is the heart of the agent-specific logic within the core.
//! It provides a comprehensive set of features for building and running AI agents,
//! including:
//! - **Configuration & Credentials**: Management of user settings and secure storage.
//! - **Agent Runtime**: Dispatchers, loops, and prompt management for agent execution.
//! - **Memory & Knowledge**: Systems for persistent storage and retrieval of information.
//! - **Channels & Providers**: Integrations with external platforms (Telegram, Discord, etc.).
//! - **Skills & Tools**: Extensible runtime for adding custom capabilities to agents.
//! - **Security & Monitoring**: Sandboxing, health checks, and audit logging.

// These modules define the public API surface for agent features.
// Many types/functions are intended for future use or integration with the frontend.
#![allow(dead_code)]

pub mod agent;
pub mod channels;
pub mod config;
pub mod cron;
pub mod desktop;
#[cfg(feature = "flows")]
pub mod flows;
pub mod hosted;
// The whole http_host domain is an axum static-directory server, so it is
// exclusive to the `http-server` feature (#5048). Its only outside reference is
// the controller-registration push in `core::all`, itself gated in lockstep, so
// no stub facade is needed — a slim build simply omits the `http_host.*` RPC
// surface (unknown-method over `/rpc`, absent from `/schema`).
#[cfg(feature = "http-server")]
pub mod http_host;
pub mod inference;
pub mod integrations;
// Vendor-neutral JSON Schema / JSON value walking, shared by the Composio
// catalog and the tinyflows capability adapters. Deliberately owned by neither:
// if it lived in either, the other would need a dependency edge into it, and
// one of those directions is the back-edge the kernelization work removes.
// Ungated — `composio` is always compiled, `tinyflows` is behind `flows`.
pub mod json_schema;
// Ungated family root: `mcp/http_client` is always compiled, and the
// `server`/`registry`/`audit` facades each need their `stub` to resolve in an
// `mcp`-less build. The gate is pushed onto each member inside `mcp/mod.rs`.
pub mod mcp;
// Both children (`generation`, `image`) are wholly gated, so the parent is a
// leaf gate — a slim build omits the family outright.
#[cfg(feature = "media")]
pub mod media;
pub mod medulla;
// The `meet` family is a facade: the parent module is always compiled so the
// always-on callers of `meet::backend_bot`'s stub keep resolving, and each
// submodule carries its own `#[cfg(feature = "meet")]` (see `meet/mod.rs`).
pub mod meet;
pub mod memory;
pub mod platform;
pub mod runtime;
pub mod sandbox;
pub mod search;
pub mod security;
pub mod skills;
pub mod subconscious;
#[cfg(feature = "e2e-test-support")]
pub mod test_support;
pub mod threads;
pub mod tinyplace;
pub mod tools;
pub mod util;
pub mod voice;
pub mod web3;
pub mod web_chat;
