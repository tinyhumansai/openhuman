//! `openhuman-core` — the in-process Rust core.
//!
//! Cargo package `openhuman`, library `openhuman_core`, binary `openhuman-core`
//! (`src/main.rs`). Owns business rules, persistence, execution
//! policy, the JSON-RPC/Socket.IO server, and the CLI. Hosted in-process by
//! `openhuman-app` (the Tauri shell), `openhuman-embed` (the typed facade for
//! third-party embedders such as Medulla and OpenCompany), and `openhuman-tui`.
//!
//! Business logic lives one directory per domain family under `src/`, listed
//! below in the order they are declared (module declarations are kept
//! alphabetical, with the `rpc` re-export sitting between `platform` and
//! `runtime` — keep new modules sorted the same way): `agent`, `api`,
//! `channels`, `config`, `core`, `cron`, `desktop`, `flows` (feature
//! `flows`), `hooks`, `hosting` (feature `hosting`), `http_host`
//! (feature `http-server`), `inference`, `integrations`, `json_schema`,
//! `mcp`, `media` (feature `media`), `medulla`, `memory`, `modules` (feature
//! `modules`), `platform`, `runtime`, `sandbox`, `search`, `security`,
//! `skills`, `test_support` (feature `e2e-test-support`), `threads`, `tools`,
//! `util`, `voice`, `web3`, `web_chat`. `channels`, `mcp`, `medulla`,
//! `skills`, `voice` and `web3` are always declared but gate most of their
//! contents inside their own `mod.rs` behind the feature of the same name.
//! `core/` is not a domain: it holds transport, dispatch, the controller
//! registry (`core::all`), auth, the CLI, the event bus, and runtime
//! composition (`core::runtime`). See `README.md` and AGENTS.md ("Rust domain
//! structure") for the preferred per-domain module shape.
//!
//! `pub use openhuman_rpc as rpc;` re-exports the `openhuman-rpc` crate, so
//! `crate::rpc::{RpcOutcome, StructuredRpcError, ...}` are the same types the
//! app and TUI decode responses with — there is no separate RPC contract
//! layer in this crate.
//!
//! [`CoreBuilder`], [`CoreRuntime`], [`DomainSet`], [`ServiceSet`],
//! [`TokenSource`] and [`HostKind`] are the embeddable composition API;
//! `openhuman-embed` layers a typed facade on top of them.
//! [`run_core_from_args`] is the CLI entry point shared by `src/main.rs` and
//! the desktop shell binary's `core` and `mcp` subcommands.
//!
//! Cargo features split into a contributor `default` set and a larger
//! shipped-product set (`scripts/ci/product-features.txt`); slim or headless
//! embedding builds use `--no-default-features --features "<list>"`. See the
//! `[features]` block in `Cargo.toml` for the full gate list and rationale.

// The RPC dispatch chokepoint wraps each handler future in an ambient
// `CoreContext` scope (Phase 2). Combined with the already very deep async type
// stacks in the axum routes that fan out into the tinyagents harness, the extra
// future layer pushes the compiler's `Send` auto-trait solver past the default
// depth of 128 (E0275). Raising the limit is the standard remedy for deep async
// type recursion and costs nothing at runtime.
#![recursion_limit = "256"]
// These modules define the public API surface for agent features.
// Many types/functions are intended for future use or integration with the frontend.
#![allow(dead_code)]

pub mod agent;
pub mod api;
pub mod channels;
pub mod config;
pub mod core;
pub mod cron;
pub mod desktop;
#[cfg(feature = "flows")]
pub mod flows;
pub mod hooks;
#[cfg(feature = "hosting")]
pub mod hosting;
#[cfg(feature = "http-server")]
pub mod http_host;
pub mod inference;
pub mod integrations;
pub mod json_schema;
pub mod mcp;
#[cfg(feature = "media")]
pub mod media;
pub mod medulla;
pub mod memory;
#[cfg(feature = "modules")]
pub mod modules;
pub mod platform;
pub use openhuman_rpc as rpc;
pub mod runtime;
pub mod sandbox;
pub mod search;
pub mod security;
pub mod skills;
#[cfg(feature = "e2e-test-support")]
pub mod test_support;
pub mod threads;
pub mod tools;
pub mod util;
pub mod voice;
pub mod web3;
pub mod web_chat;

pub use config::DaemonConfig;

/// Embeddable core composition API. Host the OpenHuman core in any process —
/// the Tauri shell, a CLI, a stdio MCP server, or a cloud/team server — via
/// [`CoreBuilder`] → [`CoreRuntime`]. See `crates/openhuman-embed/README.md`
/// and [`crate::core::runtime`] for the composition this builds on.
pub use core::runtime::{CoreBuilder, CoreRuntime, DomainSet, ServiceSet, TokenSource};
pub use core::types::HostKind;

/// Runs the core logic based on the provided command-line arguments.
///
/// This is the primary entry point for the OpenHuman binary, delegating to the
/// CLI module for argument parsing and command dispatch.
///
/// # Arguments
///
/// * `args` - A slice of strings containing the command-line arguments.
///
/// # Errors
///
/// Returns an error if command execution fails.
pub fn run_core_from_args(args: &[String]) -> anyhow::Result<()> {
    core::cli::load_dotenv_for_cli()?;
    platform::service::apply_startup_restart_delay_from_env();
    security::keyring::init_master_key();
    core::cli::run_from_cli_args(args)
}
