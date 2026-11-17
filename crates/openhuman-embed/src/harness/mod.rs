//! The one-call front door: build a harness, run turns on it.
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use openhuman_embed::{Access, Harness, Provider, Workspace};
//!
//! let harness = Harness::builder()
//!     .provider(Provider::openai_compatible("https://api.example/v1", "sk-…").model("gpt-5"))
//!     .workspace(Workspace::Ephemeral)
//!     .access(Access::readonly())
//!     // Optional: point non-inference backend calls at the embedding
//!     // product's backend. Caller-supplied inference needs no app login.
//!     .backend_url("https://my-backend.example")
//!     .build()
//!     .await?;
//!
//! let first = harness.run("Summarize what you can see.").await?;
//! println!("{}", first.reply);
//!
//! // Continue the same conversation.
//! let second = harness
//!     .turn("Now list the risks.")
//!     .session(&first.session_id)
//!     .send()
//!     .await?;
//! println!("{}", second.reply);
//! # Ok(())
//! # }
//! ```
//!
//! # What this is, relative to the rest of `embed`
//!
//! `Harness` is the one-agent convenience: a [`Runtime`](crate::Runtime)
//! plus exactly one [`Agent`](crate::Agent) named `harness`, built from one
//! set of inputs. Every method delegates to those two — [`Harness::runtime`]
//! and [`Harness::agent`] hand them out — so a host that outgrows one agent
//! creates more on the same runtime rather than a second harness.
//! [`Core`](crate::Core) is the layer below both: the typed facade over a
//! [`CoreRuntime`] the caller built themselves.
//!
//! # Running on your own endpoint
//!
//! A harness identifies as [`HostKind::Library`](openhuman_core::core::types::HostKind::Library)
//! by default. Supplying a [`Provider`] is therefore enough for inference: the
//! library host is trusted to supply its endpoint and credentials, without an
//! OpenHuman app login. Managed TinyHumans inference needs the runtime's
//! API key instead — [`RuntimeBuilder::api_key`](crate::RuntimeBuilder::api_key).
//!
//! The core can still make non-inference backend calls (integrations,
//! telemetry, managed services). [`HarnessBuilder::backend_url`] points them
//! at the embedding product's backend; [`HarnessBuilder::session`] installs a
//! backend identity when required.
//!
//! Neither applies to [`Provider::inherit`] with [`Workspace::Inherit`], which
//! runs exactly as the installed app does, session included.
//!
//! # Two things the harness cannot do for you
//!
//! **The tokio runtime is yours, and its stack size matters.** One agent turn is
//! an enormous async state machine, and delegating to a sub-agent nests another
//! inside it; tokio's default 2 MiB worker stack overflows and aborts the
//! process. Build the runtime with
//! [`AGENT_WORKER_STACK_BYTES`](openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
//! and [`MAX_BLOCKING_THREADS`](openhuman_core::core::runtime::MAX_BLOCKING_THREADS):
//!
//! ```no_run
//! use openhuman_core::core::runtime::{AGENT_WORKER_STACK_BYTES, MAX_BLOCKING_THREADS};
//!
//! let runtime = tokio::runtime::Builder::new_multi_thread()
//!     .enable_all()
//!     .thread_stack_size(AGENT_WORKER_STACK_BYTES)
//!     .max_blocking_threads(MAX_BLOCKING_THREADS)
//!     .build()
//!     .expect("tokio runtime");
//! ```
//!
//! **One runtime per process.** The keyring master key, the RPC bearer, the
//! global event bus and the `Once`-guarded domain subscribers are all
//! process-scoped. A harness owns a runtime, so a second harness is a second
//! runtime and [`HarnessBuilder::build`] returns
//! [`HarnessError::AlreadyRunning`]. Agents multiplex inside one runtime; see
//! [`Runtime`](crate::Runtime).

mod access;
mod builder;
mod error;
#[cfg(feature = "mcp")]
mod mcp;
pub(crate) mod provider;
#[cfg(feature = "skills")]
pub(crate) mod skills;
pub(crate) mod workspace;

pub use access::Access;
pub use builder::HarnessBuilder;
pub use error::HarnessError;
#[cfg(feature = "mcp")]
pub use mcp::{HttpHeader, McpAuthConfig, McpServer};
pub use provider::Provider;
pub use workspace::Workspace;

use std::path::Path;

use crate::agent::Agent;
use crate::runtime::Runtime;
use crate::turn::{Turn, TurnOutcome};
use crate::Core;

/// An embedded OpenHuman agent harness: one runtime, one agent.
///
/// Build once with [`Harness::builder`], then run as many turns as you like.
/// Dropping it tears the runtime down and, for [`Workspace::Ephemeral`],
/// removes the workspace.
pub struct Harness {
    /// Dropped before the runtime: the agent holds the core alive.
    agent: Option<Agent>,
    runtime: Option<Runtime>,
}

/// Borrowed access to the core owned by a [`Harness`] or a [`Runtime`].
///
/// Unlike [`Core`], this facade is deliberately not cloneable and exposes
/// neither the orchestrator agent facade nor the raw runtime: either path
/// could start a turn without an agent's provider route and access tier.
pub struct HarnessCore<'a> {
    core: &'a Core,
}

impl<'a> HarnessCore<'a> {
    pub(crate) fn new(core: &'a Core) -> Self {
        Self { core }
    }

    /// Read and update the runtime configuration.
    pub fn config(&self) -> crate::Config<'_> {
        self.core.config()
    }

    /// Manage the credential shared by agents on this runtime.
    pub fn auth(&self) -> crate::Auth<'_> {
        self.core.auth()
    }
}

impl Harness {
    /// Start configuring a harness.
    pub fn builder() -> HarnessBuilder {
        HarnessBuilder::new()
    }

    pub(crate) fn from_parts(runtime: Runtime, agent: Agent) -> Self {
        Self {
            agent: Some(agent),
            runtime: Some(runtime),
        }
    }

    /// The runtime this harness built. Create further agents on it with
    /// [`Runtime::agent`].
    pub fn runtime(&self) -> &Runtime {
        self.runtime
            .as_ref()
            .expect("harness runtime is present until drop")
    }

    /// The harness's own agent (id `harness`).
    pub fn agent(&self) -> &Agent {
        self.agent
            .as_ref()
            .expect("harness agent is present until drop")
    }

    /// Run one turn and get the reply.
    ///
    /// Each call starts a **new** conversation. Pass the returned
    /// [`TurnOutcome::session_id`] to [`Harness::turn`] +
    /// [`Turn::session`](crate::Turn::session) to continue one.
    pub async fn run(&self, message: impl Into<String>) -> Result<TurnOutcome, HarnessError> {
        self.turn(message).send().await.map_err(Into::into)
    }

    /// Begin a turn, to configure before sending.
    ///
    /// The harness's provider route and access origin are pre-applied; anything
    /// set on the returned [`Turn`] overrides them for that turn alone.
    pub fn turn(&self, message: impl Into<String>) -> Turn {
        self.agent().turn(message)
    }

    /// Safe typed access to non-turn core domains. Agent turns intentionally
    /// remain on [`Harness::turn`], which always applies the harness provider
    /// route and access origin.
    pub fn core(&self) -> HarnessCore<'_> {
        self.runtime().core()
    }

    /// The workspace this harness is rooted at.
    pub fn workspace_dir(&self) -> &Path {
        self.runtime().workspace_dir()
    }

    /// The agent's read/write root for acting tools.
    pub fn action_dir(&self) -> &Path {
        self.agent().action_dir()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // The agent holds an `Arc` to the core; release it first so the
        // runtime's drop can tear the core down and remove an ephemeral
        // workspace with nothing still referencing it.
        drop(self.agent.take());
        drop(self.runtime.take());
        log::debug!("[embed][harness] released");
    }
}

impl std::fmt::Debug for Harness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Resolved workspace paths and a provider bearer are both in here.
        f.debug_struct("Harness").finish_non_exhaustive()
    }
}
