//! Typed embedding facade over [`CoreRuntime`].
//!
//! Two ways in:
//!
//! * **[`Runtime`] → [`Agent`]** — the library API. Initialise one runtime
//!   (features, services, backend, the TinyHumans API key), then instantiate
//!   any number of independently configured agents on it, each with its own
//!   MCP servers, skills, working directory, access tier, provider and
//!   prompt. [`Harness`] is the one-agent shorthand over the same two types.
//! * **[`Core`]** — the typed facade over a [`CoreRuntime`] the host built
//!   itself with [`CoreBuilder`].
//!   [`CoreRuntime::invoke`] gives it JSON; this facade gives it real Rust
//!   types, so a host never writes `serde_json::json!` or matches on an
//!   error string.
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::Arc;
//! use openhuman_embed::Core;
//! use openhuman_embed::{CoreBuilder, DomainSet, HostKind, ServiceSet};
//!
//! let runtime = CoreBuilder::new(HostKind::detect_standalone())
//!     .domains(DomainSet::full())
//!     .services(ServiceSet::none())
//!     .build()
//!     .await?;
//!
//! let core = Core::from_runtime(Arc::new(runtime));
//! let flags = core.config().runtime_flags().await?;
//! println!("log_prompts={}", flags.log_prompts);
//! # Ok(())
//! # }
//! ```
//!
//! # Shape: a struct with borrowed sub-facades, not a trait
//!
//! A single trait spanning every domain would run to well over a hundred
//! methods and force each test double to stub all of them. Instead [`Core`] is
//! a concrete struct whose accessors return zero-cost newtypes over a borrow.
//!
//! Traits appear in exactly one place in this architecture — the **ports**,
//! where a host injects behaviour *downward* into the core (a workflow's
//! harness dispatcher, for instance). Facade and port are opposite directions
//! and deliberately use opposite mechanisms:
//!
//! | Direction | Mechanism |
//! |---|---|
//! | host calls core | this facade — concrete struct, typed methods |
//! | core calls host | a port — trait object installed by the host |
//!
//! # Gated domains
//!
//! Sub-facade accessors for gated domains are `#[cfg]`-compiled, so
//! `core.workflows()` simply does not exist in a build without that feature —
//! a compile error at the call site rather than a runtime surprise. For domains
//! present at compile time but switched off at runtime via
//! [`DomainSet`], calls return
//! [`CoreError::Unavailable`] so a host can hide the surface instead of
//! reporting a failure.

// The owned streaming task proves Send for the core’s deeply nested turn future.
#![recursion_limit = "256"]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

/// Complete definition accepted by [`AgentDefinitionSpec::from_base`].
pub use openhuman_core::agent::harness::definition::AgentDefinition;
pub use openhuman_core::agent::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};
pub use openhuman_core::backend::{
    install_backend_transport, installed_backend_transport, BackendRequest, BackendTransport,
    BackendTransportError, BaseUrlPurpose, TransportProfile,
};
pub use openhuman_core::config::ComposioHostCredential;
pub use openhuman_core::config::Config as RuntimeConfig;
pub use openhuman_core::security::TrustedAccess;
pub use openhuman_core::tools::toolpacks::{GroupMode, ToolGroups};
// The seam `AgentSpec::tools` needs: the belt types, and `Tool` itself from the
// vendored tinytools. An embedder that took `tinytools` as its own dependency
// would build tools of a different, incompatible type.
pub use openhuman_core::agent::tinyagents::host::LastTurnUsage;
pub use openhuman_core::agent::{HostTools, HostTurnTools, TurnContext};
pub use openhuman_core::tools::{PermissionLevel, Tool, ToolExposure, ToolResult};
pub use openhuman_core::{
    CoreBuilder, CoreRuntime, DaemonConfig, DomainSet, HostKind, ServiceSet, TokenSource,
};
/// Declarative execution requirements for host-owned tools, from the core's vendored contract.
pub use tinytools::ToolPolicy;

/// Live agent-turn progress for in-process embedders.
pub mod agent_progress {
    pub use openhuman_core::agent::progress::AgentProgress;
    pub use openhuman_core::agent::progress_sink::{
        current_progress_sink, with_progress_sink, ProgressSink, AGENT_PROGRESS_SINK,
    };
}

/// The skill registry: the host transport an embedder passes to
/// `tinyskills::SkillRegistry::builder`, and the process registry the core
/// itself serves catalog RPC and tools from.
#[cfg(feature = "skills")]
pub mod skill_registry {
    pub use openhuman_core::skills::catalog::{
        skill_registry, ReqwestTransport, HERMES_REGISTRY_ID,
    };
}

mod agent;
pub mod artifacts;
mod auth;
pub mod budget;
mod call;
#[cfg(feature = "channels")]
pub mod channels;
pub mod chat_surface;
pub mod complete;
pub mod config;
mod core_agent;
pub mod cron;
pub mod embeddings;
mod error;
pub mod fanout;
mod harness;
pub mod identity;
pub mod memory;
pub mod modules;
mod permission;
pub use permission::PermissionFuture;
pub mod observe;

pub mod process;
#[cfg(feature = "channels")]
pub mod profiles;
/// Explicit ordered fallback and truncation policies.
pub mod routing;
mod runtime;
mod turn;
mod turn_cancellation;
mod turn_meter;

/// Core internals for `openhuman-tinyhumans` and `openhuman-rpc` only; see
/// the module docs. Not part of the host-facing API.
#[doc(hidden)]
#[path = "host_internals.rs"]
pub mod __host;

/// Look up the schema of a registered RPC method (`openhuman.<ns>_<fn>`).
pub use openhuman_core::core::all::schema_for_rpc_method;
/// Whether this build carries the HTTP server surface the router mounts.
pub use openhuman_core::core::http_server_status::HTTP_SERVER_COMPILED_IN;
pub use openhuman_core::core::ControllerSchema;
/// Why a listen-port pick failed (another live core holds the port, ...).
pub use openhuman_core::platform::connectivity::rpc::PickListenPortError;
/// Whether this build carries the voice domain.
pub use openhuman_core::voice::VOICE_COMPILED_IN;

pub use agent::ToolAttachmentError;
pub use agent::{
    Agent, AgentDefinitionSpec, AgentError, AgentLayout, AgentSpec, ApprovalDecision, Approvals,
    ApprovalsError, DefinitionBase, MemoryBinding, PendingApproval, SandboxModeSpec, ToolScopeSpec,
};
pub use auth::{Auth, AuthState, Session};
#[cfg(feature = "channels")]
pub use channels::{ChannelError, ChannelListener, Channels, StreamMode, TelegramChannelSpec};
pub use config::{Config, RuntimeFlags};
pub use core_agent::CoreAgent;
pub use cron::{
    Cron, CronError, JobRun, JobRunRecord, JobSchedule, JobSpec, JobTarget, ScheduledJob,
    SystemJobContext,
};
pub use error::CoreError;
pub use harness::{
    Access, Harness, HarnessBuilder, HarnessCore, HarnessError, Provider, Workspace,
};
#[cfg(feature = "mcp")]
pub use harness::{HttpHeader, McpAuthConfig, McpServer};
#[cfg(feature = "channels")]
pub use profiles::{
    ChatReply, OpenError, ProfileError, ProfileEvents, ProfileHandle, ProfileId, ProfileIdMode,
    ProfileRuntime, ProfileRuntimeBuilder, ProfileSummary, Provisioned, RelayAccepted,
    RelayMessage, SaasConfig,
};
pub use runtime::builder::DEFAULT_MAX_AGENTS;
/// Read-only view of a [`RuntimeBuilder`], for the layered crates' tests.
#[doc(hidden)]
pub use runtime::BuilderSummary;
pub use runtime::{
    run_from_args, AgentDefaults, ApiKey, ConfigSource, ConfigurationInfo, DefaultsInfo,
    LearningSettings, ModelDefaults, RemoveAgent, Runtime, RuntimeBuilder, RuntimeDefaults,
    RuntimeError, RuntimeInfo, RuntimeModule, SkillsPolicy, StorageInfo, WeightClass,
};

/// The types the [`RuntimeBuilder`] seam options take: controller
/// extensions, embedder hooks, the CLI server launcher and the live security
/// policy. A `tool_search` ranker implements `tinytools::ToolRanker`; take
/// `tinytools` from the vendored path (`vendor/tinyagents/vendor/tinytools`)
/// so the trait unifies — re-exporting it here would break the
/// agent-runtime ownership boundary.
pub mod seams {
    pub use openhuman_core::agent::hooks::{PostTurnHook, ToolHook};
    pub use openhuman_core::agent::hooks::{ToolHookContext, ToolHookDecision, TurnContext};
    pub use openhuman_core::agent::stop_hooks::{
        BudgetStopHook, StopDecision, StopHook, TurnState,
    };
    pub use openhuman_core::core::all::{ControllerExtension, DomainGroup};
    pub use openhuman_core::core::server_launcher::{HostBoot, ServeRequest, ServerLauncher};
    pub use openhuman_core::security::SecurityPolicy;
    pub use openhuman_core::storage::{CollectionSpec, Precondition, Scope, StorageBackend};

    pub use crate::runtime::StorageSource;
}

/// The session store port: what a host implements to keep every agent's
/// conversations in its own database ([`RuntimeBuilder::session_store`]),
/// with the in-memory provider and the conformance suites a host's provider
/// is held to.
pub mod session_store {
    pub use openhuman_core::agent::session_store::{AgentStores, SessionStoreProvider};
    pub use tinyagents_session::port::{
        AppendStore, InMemorySessionStores, InMemoryTranscriptLocator, InMemoryTurnStates, Store,
        TurnStates,
    };
    pub use tinyagents_session::testkit::conformance::{
        session_store_conformance, session_store_isolation_conformance,
    };
    /// The transcript seam a provider's [`AgentStores::transcripts`] implements.
    pub use tinyagents_session::transcript::{
        SessionRef, SessionTranscript, TranscriptHistory, TranscriptLocator, TranscriptMessage,
        TranscriptMeta, TranscriptPartial, TranscriptRead, TranscriptTurn,
    };
    /// The turn snapshots a provider's [`AgentStores::turn_states`] keeps.
    pub use tinyagents_session::turn_state::{TurnLifecycle, TurnState};
    /// The error and result the key-value and journal seams return.
    pub use tinyagents_session::{Result as StoreResult, TinyAgentsError as StoreError};
}
pub use complete::{
    ChatMessage, Completer, CompletionObserver, CompletionRequest, CompletionResponse,
    CompletionTrace, CompletionUsage,
};
pub use session_store::{InMemorySessionStores, SessionStoreProvider};
pub use turn::{absolute, Route, Turn, TurnOutcome, TurnRequest};
pub use turn_cancellation::TurnCancellation;

use std::sync::Arc;

/// Typed handle to an embedded OpenHuman core.
///
/// Cheap to clone: it is an `Arc` over the runtime the embedder already owns.
#[derive(Clone)]
pub struct Core {
    rt: Arc<CoreRuntime>,
}

impl Core {
    /// Wrap an already-built [`CoreRuntime`].
    ///
    /// The runtime must come from
    /// [`CoreBuilder::build`](openhuman_core::core::runtime::CoreBuilder::build). The
    /// facade neither starts nor stops background services; lifecycle stays
    /// with the embedder.
    pub fn from_runtime(rt: Arc<CoreRuntime>) -> Self {
        log::debug!("[embed] core_facade_attached services={:?}", rt.services());
        Self { rt }
    }

    /// Typed configuration access.
    pub fn config(&self) -> Config<'_> {
        Config(&self.rt)
    }

    /// Typed access to the session store.
    ///
    /// Use this when the embedded workload calls authenticated TinyHumans
    /// backend services. A [`HostKind::Library`]
    /// runtime does not need an app session for caller-supplied inference.
    pub fn auth(&self) -> Auth<'_> {
        Auth(&self.rt)
    }

    /// Typed access to the agent harness — run a turn on the runtime's
    /// orchestrator, get a reply.
    ///
    /// Requires the `inference` domain family at runtime; with it off the turn
    /// returns [`CoreError::Unavailable`], because the routed chat entry point
    /// is registered under that group. Note
    /// [`DomainSet::harness`](openhuman_core::core::runtime::DomainSet::harness) leaves
    /// `inference` **off** despite its name — use
    /// [`DomainSet::embedded`](openhuman_core::core::runtime::DomainSet::embedded), or
    /// set the field.
    ///
    /// For agents of your own — several, each with its own MCP servers,
    /// skills, working directory and access tier — see [`Runtime::agent`].
    pub fn agent(&self) -> CoreAgent<'_> {
        CoreAgent(&self.rt)
    }

    /// The underlying runtime, for anything this facade does not yet model.
    ///
    /// An escape hatch, not the intended path — every use is a candidate for a
    /// real typed method. Callers own the JSON and the error strings.
    pub fn raw(&self) -> &Arc<CoreRuntime> {
        &self.rt
    }
}

impl std::fmt::Debug for Core {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // CoreRuntime holds resolved workspace paths and host identity; keep
        // them out of logs and panic messages.
        f.debug_struct("Core").finish_non_exhaustive()
    }
}

/// Strict structured output failure metadata.
pub mod structured;

/// Acknowledged cancellation for stateless completion operations.
pub mod cancellation;
/// Runtime event subscriptions without content or credentials.
pub mod events;
/// Owned streaming turns and cooperative cancellation.
pub mod stream;
pub use agent::{ApprovalHandler, ApprovalSubscription};
pub use events::{EventStreamError, RuntimeEvent, RuntimeEventKind, RuntimeEvents};
pub use stream::{CancellationToken, StreamEvent, TurnStream};
/// Native custom-provider contract and request/response types.
pub mod providers {
    pub use tinyinference_llm::message::MessageDelta;
    pub use tinyinference_llm::model::{
        ChatModel, DeferredHandle, DeferredStatus, ModelProfile, ModelRequest, ModelResponse,
        ModelStream, ModelStreamItem, ModelStreamMetadata,
    };
    pub use tinyinference_llm::{Error, Result};
}

pub use agent_progress::AgentProgress;
