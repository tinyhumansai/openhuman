//! The one-call front door: build a harness, run turns on it.
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use openhuman_core::{Access, Harness, Provider, Workspace};
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
//! [`Core`](crate::embed::Core) is the typed facade over a [`CoreRuntime`] the
//! caller already built. `Harness` is the layer above: it *builds* that runtime
//! from typed inputs, owns the workspace's lifetime, and applies the harness's
//! own provider and access defaults to every turn. A host that already has a
//! `CoreRuntime` — the desktop shell, an existing embedder — wants
//! `Core::agent()` and should skip this module entirely.
//!
//! # Running on your own endpoint
//!
//! A harness identifies as [`HostKind::Library`](crate::core::types::HostKind::Library)
//! by default. Supplying a [`Provider`] is therefore enough for inference: the
//! library host is trusted to supply its endpoint and credentials, without an
//! OpenHuman app login.
//!
//! The core can still make non-inference backend calls (integrations,
//! telemetry, managed services). Those need their own real session when the
//! endpoint requires one. [`HarnessBuilder::backend_url`] points them at the
//! embedding product's backend; [`HarnessBuilder::session`] installs a backend
//! identity when required.
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
//! [`AGENT_WORKER_STACK_BYTES`](crate::core::runtime::AGENT_WORKER_STACK_BYTES)
//! and [`MAX_BLOCKING_THREADS`](crate::core::runtime::MAX_BLOCKING_THREADS):
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
//! **One harness per process.** The keyring master key, the RPC bearer, the
//! global event bus and the `Once`-guarded domain subscribers are all
//! process-scoped, so two harnesses would silently share them while believing
//! they had separate workspaces. [`HarnessBuilder::build`] returns
//! [`HarnessError::AlreadyRunning`] rather than letting that happen. Lifting the
//! restriction is phase 3 of `docs/plans/pluggable-core/`.

mod access;
mod builder;
mod error;
#[cfg(feature = "mcp")]
mod mcp;
mod provider;
#[cfg(feature = "skills")]
mod skills;
mod workspace;

pub use access::Access;
pub use builder::HarnessBuilder;
pub use error::HarnessError;
#[cfg(feature = "mcp")]
pub use mcp::{HttpHeader, McpAuthConfig, McpServer};
pub use provider::Provider;
pub use workspace::Workspace;

use std::path::Path;
use std::sync::atomic::AtomicBool;

use workspace::ResolvedWorkspace;

use crate::embed::agent::{Turn, TurnOutcome};
use crate::embed::Core;

/// Guards the process-scoped core state described in the module docs.
static HARNESS_LIVE: AtomicBool = AtomicBool::new(false);

/// An embedded OpenHuman agent harness.
///
/// Build once with [`Harness::builder`], then run as many turns as you like.
/// Dropping it releases the process slot and, for
/// [`Workspace::Ephemeral`], removes the workspace.
pub struct Harness {
    core: Option<Core>,
    provider: Provider,
    access: Access,
    /// Held for its `Drop`: an ephemeral workspace lives exactly as long as the
    /// harness that owns it.
    _workspace: ResolvedWorkspace,
}

/// Borrowed access to the core owned by a [`Harness`].
///
/// Unlike [`Core`], this facade is deliberately not cloneable and exposes
/// neither the unconfigured agent facade nor the raw runtime: either path
/// could start a turn without the harness's caller-supplied provider route.
/// Share an `Arc<Harness>` when several agents need concurrent turn access.
pub struct HarnessCore<'a> {
    core: &'a Core,
}

impl HarnessCore<'_> {
    pub fn config(&self) -> crate::embed::Config<'_> {
        self.core.config()
    }

    pub fn auth(&self) -> crate::embed::Auth<'_> {
        self.core.auth()
    }

    #[cfg(feature = "medulla")]
    pub fn medulla(&self) -> crate::embed::Medulla<'_> {
        self.core.medulla()
    }
}

impl Harness {
    /// Start configuring a harness.
    pub fn builder() -> HarnessBuilder {
        HarnessBuilder::new()
    }

    /// Run one turn and get the reply.
    ///
    /// Each call starts a **new** conversation. Pass the returned
    /// [`TurnOutcome::session_id`] to [`Harness::turn`] +
    /// [`Turn::session`](crate::embed::Turn::session) to continue one.
    pub async fn run(&self, message: impl Into<String>) -> Result<TurnOutcome, HarnessError> {
        self.turn(message).send().await.map_err(Into::into)
    }

    /// Begin a turn, to configure before sending.
    ///
    /// The harness's provider route and access origin are pre-applied; anything
    /// set on the returned [`Turn`] overrides them for that turn alone.
    pub fn turn(&self, message: impl Into<String>) -> Turn<'_> {
        let mut turn = self
            .core
            .as_ref()
            .expect("harness core is present until drop")
            .agent()
            .turn(message);
        if let Some(route) = self.provider.route() {
            turn = turn.route(route.clone());
        }
        if let Some(model) = self.provider.model_id() {
            turn = turn.model(model);
        }
        if let Some(origin) = self.access.turn_origin() {
            turn = turn.origin(origin.clone());
        }
        turn
    }

    /// Safe typed access to non-turn core domains. Agent turns intentionally
    /// remain on [`Harness::turn`], which always applies the harness provider
    /// route and access origin.
    pub fn core(&self) -> HarnessCore<'_> {
        HarnessCore {
            core: self
                .core
                .as_ref()
                .expect("harness core is present until drop"),
        }
    }

    /// The workspace this harness is rooted at.
    ///
    /// Empty for [`Workspace::Inherit`], where the operator's own resolution
    /// decides and the harness never computes a path of its own.
    pub fn workspace_dir(&self) -> &Path {
        &self._workspace.workspace_dir
    }

    /// The agent's read/write root for acting tools.
    pub fn action_dir(&self) -> &Path {
        &self._workspace.action_dir
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Drop the old core while the process slot is still claimed. Releasing
        // it first lets another builder initialize process-scoped state while
        // this runtime's keyring, bearer, event bus and subscribers are live.
        drop(self.core.take());
        // For an ephemeral workspace, take ownership of the temp path and
        // remove it with a short retry. The core's memory/session writers keep
        // running a moment after the harness returns from a turn and can
        // recreate workspace subdirectories while `TempDir`'s own drop-time
        // removal is racing them, leaving an empty directory behind and
        // breaking the documented "removed with its harness" guarantee. A
        // bounded retry lets those writes settle before we give up.
        if let Some(temp) = self._workspace._temp.take() {
            // `keep()` hands back the path without removing the directory so
            // we can do the retried removal ourselves.
            let root = temp.keep();
            // Require a short quiet period rather than trusting one successful
            // removal: a detached session writer can recreate the directory
            // immediately afterward. Most drops finish in ~200 ms; repeated
            // writes retain the one-second hard cap.
            let mut quiet_passes = 0;
            for _ in 0..20 {
                match std::fs::remove_dir_all(&root) {
                    Ok(()) => quiet_passes += 1,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        quiet_passes += 1;
                    }
                    Err(_) => quiet_passes = 0,
                }
                if quiet_passes >= 5 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let _ = std::fs::remove_dir_all(&root);
        }
        HARNESS_LIVE.store(false, std::sync::atomic::Ordering::Release);
        log::debug!("[embed][harness] released");
    }
}

impl std::fmt::Debug for Harness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Resolved workspace paths and a provider bearer are both in here.
        f.debug_struct("Harness").finish_non_exhaustive()
    }
}
