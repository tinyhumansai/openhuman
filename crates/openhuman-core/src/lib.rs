//! `openhuman-core` — the in-process Rust core.
//!
//! Cargo package `openhuman`, library `openhuman_core`, binary `openhuman-core`
//! (`src/main.rs`). Owns business rules, persistence, execution
//! policy, the JSON-RPC/Socket.IO server, and the CLI. Hosted in-process by
//! `openhuman-app` (the Tauri shell), `openhuman-embed` (the typed facade for
//! third-party embedders such as OpenCompany), and `openhuman-tui`.
//!
//! Business logic lives one directory per domain family under `src/`, listed
//! below in the order they are declared (module declarations are kept
//! alphabetical — keep new modules sorted the same way): `agent`,
//! `agent_registry`, `agent_runs`, `artifacts`, `attachments`, `channels`,
//! `commands`, `config`, `core`, `credentials`, `cron`, `desktop`, `doctor`,
//! `flows` (feature `flows`), `hooks`, `hosting` (feature `hosting`),
//! `inbound` (feature `inbound-queues`), `inbound_debounce`, `inference`,
//! `integrations`, `mcp`, `media` (feature `media`), `memory`, `modules`,
//! `plan_review`, `platform`, `profiles`, `progress`, `prompts`, `sandbox`,
//! `search`, `security`, `sessions`, `skills`, `storage`, `test_support`
//! (feature `e2e-test-support`), `threads`, `tools`, `triage`, `util`,
//! `voice`, `web3`, `web_chat`. `channels`, `mcp`, `modules`, `skills`,
//! `voice` and `web3` are always declared but gate most of their contents
//! inside their own `mod.rs` behind the feature of the same name.
//! `core/` is not a domain: it holds transport, dispatch, the controller
//! registry (`core::all`), auth, the CLI, the event bus, and runtime
//! composition (`core::runtime`). See `README.md` and AGENTS.md ("Rust domain
//! structure") for the preferred per-domain module shape.
//!
//! The controller contract lives in `core`: every domain operation returns
//! [`core::Outcome`], controllers are described by `core::ControllerSchema`,
//! and `core::invoke::invoke_method` dispatches them in-process. The JSON-RPC
//! protocol and server that expose them live in `openhuman-rpc`, which depends
//! on this crate.
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
// TODO(#335): hundreds of items across most modules still rely on this; `tools`
// opts back into the lint below. Remove it module by module.
#![allow(dead_code)]

pub mod agent;
pub mod agent_registry;
pub mod agent_runs;
pub mod artifacts;
pub mod attachments;
// These host adapters still call TinyChannels' v21 compatibility surface.
// Keep the deprecation exception scoped here while the v22 adapter migration
// is handled separately.
#[allow(deprecated)]
pub mod channels;
pub mod commands;
pub mod config;
pub mod core;
pub mod credentials;
// These host adapters still call TinyChannels' v21 compatibility surface.
// Keep the deprecation exception scoped here while the v22 adapter migration
// is handled separately.
#[allow(deprecated)]
pub mod cron;
pub mod desktop;
pub mod doctor;
#[cfg(feature = "flows")]
pub mod flows;
pub mod hooks;
#[cfg(feature = "hosting")]
pub mod hosting;
#[cfg(feature = "inbound-queues")]
pub mod inbound;
pub mod inbound_debounce;
pub mod inference;
pub mod integrations;
pub mod mcp;
#[cfg(feature = "media")]
pub mod media;
pub mod memory;
pub mod modules;
pub mod plan_review;
pub mod platform;
pub mod profiles;
pub mod progress;
pub mod prompts;
pub mod sandbox;
pub mod search;
pub mod security;
pub mod sessions;
pub mod skills;
pub mod storage;
#[cfg(feature = "e2e-test-support")]
pub mod test_support;
// These host adapters still call TinyChannels' v21 compatibility surface.
// Keep the deprecation exception scoped here while the v22 adapter migration
// is handled separately.
#[allow(deprecated)]
pub mod threads;
// `tools` is kept dead-code clean: re-enable the lint the crate-wide allow silences.
#[warn(dead_code)]
pub mod tools;
pub mod triage;
pub mod util;
pub mod voice;
pub mod web3;
pub mod web_chat;

/// Embeddable core composition API. Host the OpenHuman core in any process —
/// the Tauri shell, a CLI, a stdio MCP server, or a cloud/team server — via
/// [`CoreBuilder`] → [`CoreRuntime`]. See `crates/openhuman-embed/README.md`
/// and [`crate::core::runtime`] for the composition this builds on.
pub use core::runtime::{
    CoreBootMode, CoreBuilder, CoreRuntime, DomainSet, ServiceSet, TokenSource,
};
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
    run_core_from_args_with(args, None)
}

/// [`run_core_from_args`] with a host-supplied boot description.
///
/// `host_boot` rides on the `run` / `serve` [`ServeRequest`] to the installed
/// server launcher, which boots from it instead of its own default preset.
/// Explicit command-line flags still apply on top of it; see [`ServeRequest`]
/// for the precedence. Subcommands that do not start a server ignore it.
///
/// # Errors
///
/// Returns an error if command execution fails.
pub fn run_core_from_args_with(
    args: &[String],
    host_boot: Option<core::server_launcher::HostBoot>,
) -> anyhow::Result<()> {
    core::cli::load_dotenv_for_cli()?;
    platform::service::apply_startup_restart_delay_from_env();
    // A SaaS boot roots secret storage under its operator directory
    // (`saas::build`), so it must not be latched to the home directory here.
    if !core::cli::is_help_only(args)
        && !core::runtime::mode::requested_in(args, std::env::var("OPENHUMAN_MODE").ok().as_deref())
    {
        security::secret_storage::init_master_key().map_err(anyhow::Error::msg)?;
    }
    core::cli::run_from_cli_args_with(args, host_boot)
}
