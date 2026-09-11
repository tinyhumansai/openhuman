//! Core library for the OpenHuman platform.
//!
//! This crate provides the central logic for the OpenHuman core binary, including:
//! - API and RPC handlers for external interactions.
//! - Core system services (CLI, configuration, monitoring).
//! - Domain-specific logic for the OpenHuman agent runtime.

// The RPC dispatch chokepoint wraps each handler future in an ambient
// `CoreContext` scope (Phase 2). Combined with the already very deep async type
// stacks in the axum routes that fan out into the tinyagents harness, the extra
// future layer pushes the compiler's `Send` auto-trait solver past the default
// depth of 128 (E0275). Raising the limit is the standard remedy for deep async
// type recursion and costs nothing at runtime.
#![recursion_limit = "256"]

pub mod api;
pub mod core;
pub mod embed;
pub mod openhuman;
pub mod rpc;
#[cfg(feature = "tui")]
pub mod tui;

pub use openhuman::config::DaemonConfig;

/// Embeddable core composition API. Host the OpenHuman core in any process —
/// the Tauri shell, a CLI, a stdio MCP server, or a cloud/team server — via
/// [`CoreBuilder`] → [`CoreRuntime`]. See `docs/plans/pluggable-core/`.
pub use core::runtime::{CoreBuilder, CoreRuntime, DomainSet, ServiceSet, TokenSource};
pub use core::types::HostKind;

#[cfg(feature = "mcp")]
pub use embed::McpServer;
/// Run the OpenHuman agent harness as a library call.
///
/// [`CoreBuilder`] composes a core; [`embed::Core`] gives it typed methods.
/// [`Harness`] is the front door above both: configure a provider, a workspace,
/// an access tier — and MCP servers and skills where those features are
/// compiled in — then run turns.
///
/// ```no_run
/// # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
/// use openhuman_core::{Harness, Provider, Workspace};
///
/// let harness = Harness::builder()
///     .provider(Provider::openai_compatible("https://api.example/v1", "sk-…").model("gpt-5"))
///     .workspace(Workspace::Ephemeral)
///     .build()
///     .await?;
///
/// println!("{}", harness.run("Say hello.").await?.reply);
/// # Ok(())
/// # }
/// ```
///
/// Read the `embed::harness` module docs before using it: they cover the two
/// things it cannot do for you (size the tokio runtime's worker stacks, and
/// share a process with a second harness) and what running on your own
/// inference endpoint additionally needs.
pub use embed::{Access, Harness, HarnessBuilder, HarnessError, Provider, Workspace};
pub use embed::{Session, Turn, TurnOutcome, TurnRequest};

/// Live agent-turn progress for **in-process embedders**.
///
/// An embedder that drives a turn through an RPC returning a single final
/// string (`openhuman.inference_agent_chat`) sees nothing while the turn runs.
/// Scope a [`ProgressSink`](agent_progress::ProgressSink) around the future it
/// awaits and the entry point attaches it to the agent it builds internally, so
/// tool calls, deltas and turn boundaries stream out live:
///
/// ```no_run
/// # async fn demo() {
/// let (tx, mut rx) = tokio::sync::mpsc::channel(256);
/// openhuman_core::agent_progress::with_progress_sink(tx, async {
///     // drive `agent_chat` here
/// })
/// .await;
/// # let _ = rx.recv();
/// # }
/// ```
///
/// This is the whole public surface — nothing here requires reaching through
/// private modules.
pub mod agent_progress {
    pub use crate::openhuman::agent::progress::AgentProgress;
    pub use crate::openhuman::agent::progress_sink::{
        current_progress_sink, with_progress_sink, ProgressSink, AGENT_PROGRESS_SINK,
    };
}

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
    openhuman::platform::service::apply_startup_restart_delay_from_env();
    openhuman::security::keyring::init_master_key();
    core::cli::run_from_cli_args(args)
}
