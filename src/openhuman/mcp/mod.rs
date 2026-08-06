//! MCP family — Model Context Protocol client and server support.
//!
//! One directory, one gate. Every member of this family is compiled out
//! together by the default-ON `mcp` Cargo feature, with two deliberate
//! carve-outs called out below.
//!
//! # Members
//!
//! - [`server`]         — the `openhuman mcp` stdio/HTTP server that exposes a
//!   curated OpenHuman tool surface to MCP hosts (facade + `stub`).
//! - [`registry`]       — the **dynamic**, user-installed Smithery servers:
//!   live connection map, SQLite persistence, boot spawn, supervisor, OAuth,
//!   and the `mcp_clients` RPC namespace (facade + `stub`).
//! - [`audit`]          — the persistent write-audit log (facade + `stub`).
//! - [`config_servers`] — the **static**, config-declared server set read from
//!   `[[mcp_client.servers]]` in TOML, plus the stdio transport and the
//!   `mcp_setup` agent. Leaf-gated: every consumer is itself gated.
//! - [`http_client`]    — [`http_client::McpHttpClient`], the Streamable-HTTP +
//!   OAuth + SSE transport primitive. **Always compiled** (see below).
//!
//! > **The naming is inverted from intuition.** `registry` is the *dynamic*
//! > half (SQLite-backed installs); `config_servers` is the *static* half
//! > (TOML-declared). The RPC namespace and db filename remain `mcp_clients`
//! > for backwards compatibility regardless of module path.
//!
//! # Compile-time gate (`mcp` feature)
//!
//! **`pub mod mcp;` is ALWAYS compiled — the family root is a facade.** It
//! cannot carry `#[cfg(feature = "mcp")]` for two independent reasons:
//!
//! 1. [`http_client`] is a mis-housed shared utility with always-compiled
//!    consumers that have nothing to do with MCP. The bespoke `gitbooks` docs
//!    tool dials [`http_client::McpHttpClient`] + [`http_client::redact_endpoint`]
//!    directly (GitBook is modelled as a legacy MCP server), and
//!    `core::observability` names [`http_client::McpUnauthorizedError`] in its
//!    always-compiled `McpServerNeedsAuth` classifier coupling test. Gating it
//!    would break a docs tool in slim builds and leak wording drift into the
//!    classifier.
//! 2. `server`, `registry`, and `audit` each ship a `stub.rs` that must resolve
//!    in an `mcp`-less build, which requires their parent to stay compiled.
//!
//! So the gate is pushed **down onto each member**: the three facades keep
//! their own internal `#[cfg]` + `stub`, `config_servers` is leaf-gated here,
//! and `http_client` stays ungated. The same rule the `meet/` pilot proved.
//!
//! The gate follows the real dependency graph, not the directory name.
//!
//! # Ungated type carve-outs
//!
//! `registry::types`, `audit::types`, and `server::tools::types`
//! ([`server::McpToolSpec`]) stay compiled in **both** builds: they are inert
//! `serde`/`serde_json`-only data consumed by always-compiled callers (the
//! orchestrator prompt builder, `tool_registry`). Both builds share the one
//! real definition, so struct fields cannot drift between them — the stubs
//! carry behaviour only.
//!
//! # Dependencies
//!
//! The `mcp` gate sheds **zero** dependencies. There is no MCP SDK in this
//! crate: the entire protocol stack is hand-rolled over tokio process stdio +
//! `reqwest` + `axum`, all of which are load-bearing for non-MCP domains. The
//! `mcp = []` feature list is intentionally empty — do not "fix" it.

pub mod audit;
#[cfg(feature = "mcp")]
pub mod config_servers;
pub mod http_client;
pub mod registry;
pub mod server;
