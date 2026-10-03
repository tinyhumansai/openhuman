//! A runtime-owned agent: one fully described, independently configured
//! participant on a [`Runtime`](crate::Runtime).
//!
//! An [`Agent`] is a cheap clone handle over the state
//! [`Runtime::agent`](crate::Runtime::agent) assembled from an
//! [`AgentSpec`]: its own `Config` (provider route and model, MCP servers,
//! autonomy tier, `action_dir`), its
//! [`AgentDefinition`](openhuman_core::agent::harness::definition::AgentDefinition)
//! (system prompt, tool scope, sandbox mode), its
//! derived
//! [`CoreContext`](openhuman_core::core::runtime::CoreContext) every turn
//! dispatches under. Two agents on one runtime never read each other's
//! settings: each turn is scoped to its own context, and the core's config
//! loader, DomainSet gate, tool-group filter and skill discovery all read
//! the ambient context.

mod attachments;
pub use attachments::ToolAttachmentError;
pub(crate) mod build;
mod definition;
mod layout;
pub(crate) mod spec;

pub use definition::{AgentDefinitionSpec, SandboxModeSpec, ToolScopeSpec};
pub use layout::AgentLayout;
pub use spec::AgentSpec;

use std::path::Path;
use std::sync::Arc;

use openhuman_core::agent::harness::definition::AgentDefinition;
use openhuman_core::config::Config;
use openhuman_core::core::runtime::{CoreContext, CoreRuntime};

use crate::harness::{Access, Provider};
use crate::turn::{Turn, TurnOutcome, TurnTarget};
use crate::CoreError;

/// Error from instantiating or driving an [`Agent`].
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// A turn failed. See [`CoreError`] for the distinctions.
    #[error(transparent)]
    Call(#[from] CoreError),

    /// An agent with this id is already alive on the runtime.
    #[error("agent id {0:?} is already registered on this runtime")]
    DuplicateId(String),

    /// The id does not match `^[a-z0-9][a-z0-9_-]{0,63}$`.
    #[error("invalid agent id {id:?}: {reason}")]
    InvalidId {
        /// The offending id.
        id: String,
        /// Why it was refused.
        reason: String,
    },

    /// The spec asked for a domain family or tool group the runtime did not
    /// register. Agents can only narrow the runtime's surface.
    #[error("agent widens the runtime's surface: {0}")]
    WidensRuntime(String),

    /// A filesystem operation laying out the agent failed.
    #[error("failed to {what}")]
    Workspace {
        /// What was being attempted, phrased to complete "failed to …".
        what: &'static str,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// A spec input could not be honoured.
    #[error("{0}")]
    Invalid(String),
}

/// The assembled state behind an [`Agent`], shared by every clone of it and
/// by every [`Turn`] it issues.
pub(crate) struct AgentInner {
    pub(crate) id: String,
    pub(crate) runtime_id: String,
    pub(crate) attachments: attachments::Attachments,
    /// Keeps the runtime's core and (for an ephemeral workspace) its
    /// directory alive for as long as this agent is, even after the host
    /// drops its `Runtime` handle. See
    /// [`CoreGuard`](crate::runtime::CoreGuard).
    _runtime_guard: Arc<crate::runtime::CoreGuard>,
    pub(crate) runtime: Arc<CoreRuntime>,
    pub(crate) ctx: Arc<CoreContext>,
    pub(crate) config: Config,
    pub(crate) definition: AgentDefinition,
    pub(crate) provider: Provider,
    pub(crate) access: Access,
    pub(crate) layout: AgentLayout,
    /// The agent's own in-process tools, rebuilt per turn. See
    /// [`AgentSpec::tools`](super::AgentSpec::tools) for why it is a factory.
    pub(crate) host_tools: Option<openhuman_core::agent::HostTools>,
}

/// A handle to one agent on a runtime.
///
/// Cheap to clone; every clone drives the same agent. The id is released for
/// reuse once the last clone is dropped. Dropping an agent does **not** remove
/// its directories — its transcripts and memory persist with the runtime's
/// workspace.
#[derive(Clone)]
pub struct Agent {
    inner: Arc<AgentInner>,
}

impl Agent {
    pub(crate) fn from_inner(inner: Arc<AgentInner>) -> Self {
        Self { inner }
    }

    /// The id this agent was created with.
    pub fn id(&self) -> &str {
        &self.inner.id
    }

    /// Run one turn and get the reply.
    ///
    /// Each call starts a **new** conversation. Pass the returned
    /// [`TurnOutcome::session_id`] to [`Agent::turn`] + [`Turn::session`] to
    /// continue one.
    pub async fn run(&self, message: impl Into<String>) -> Result<TurnOutcome, AgentError> {
        self.turn(message).send().await.map_err(Into::into)
    }

    /// Begin a turn, to configure before sending.
    ///
    /// The agent's provider route, model and access origin are pre-applied;
    /// anything set on the returned [`Turn`] overrides them for that turn
    /// alone.
    pub fn turn(&self, message: impl Into<String>) -> Turn {
        let mut turn = Turn::new(TurnTarget::Agent(Arc::clone(&self.inner)), message)
            .with_agent_id(&self.inner.id);
        if let Some(route) = self.inner.provider.route() {
            turn = turn.route(route.clone());
        }
        if let Some(model) = self.inner.provider.model_id() {
            turn = turn.model(model);
        }
        if let Some(origin) = self.inner.access.turn_origin() {
            turn = turn.origin(origin.clone());
        }
        turn
    }

    /// The agent's read/write root for acting tools.
    pub fn action_dir(&self) -> &Path {
        &self.inner.config.action_dir
    }

    /// The runtime-wide workspace this agent's state lives under.
    pub fn workspace_dir(&self) -> &Path {
        &self.inner.config.workspace_dir
    }

    /// `<workspace>/agents/<id>/` — the agent's home (SOUL.md, MEMORY.md).
    pub fn home_dir(&self) -> &Path {
        &self.inner.layout.home
    }

    /// `<workspace>/agents/<id>/skills/` — where its skill bundles were
    /// copied and the only skills root it sees besides the workspace's own.
    pub fn skills_dir(&self) -> &Path {
        &self.inner.layout.skills
    }

    /// Where this agent's transcripts are written: the workspace's shared
    /// `session_raw/` (files are keyed by agent name), or
    /// `session_raw/`, keyed by agent id.
    pub fn transcripts_dir(&self) -> &Path {
        &self.inner.layout.transcripts
    }

    /// The provider this agent's turns run on.
    pub fn provider(&self) -> &Provider {
        &self.inner.provider
    }

    /// The access tier this agent's turns run with.
    pub fn access(&self) -> &Access {
        &self.inner.access
    }

    /// The config this agent was instantiated with, for inspection.
    pub fn config(&self) -> &Config {
        &self.inner.config
    }
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent")
            .field("id", &self.inner.id)
            .finish_non_exhaustive()
    }
}
