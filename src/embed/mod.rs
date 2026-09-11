//! Typed embedding facade over [`CoreRuntime`].
//!
//! [`CoreBuilder`](crate::core::runtime::CoreBuilder) gives an embedder a
//! running core; [`CoreRuntime::invoke`] gives it JSON. This module is the
//! third piece: real Rust types, so a host application never writes
//! `serde_json::json!` or matches on an error string.
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::Arc;
//! use openhuman_core::embed::Core;
//! use openhuman_core::{CoreBuilder, DomainSet, HostKind, ServiceSet};
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
//! [`DomainSet`](crate::core::runtime::DomainSet), calls return
//! [`CoreError::Unavailable`] so a host can hide the surface instead of
//! reporting a failure.

mod agent;
mod auth;
mod call;
mod config;
mod error;
mod harness;
#[cfg(feature = "medulla")]
mod medulla;

pub use agent::{absolute, Agent, Route, Turn, TurnOutcome, TurnRequest};
pub use auth::{Auth, AuthState, Session};
pub use config::{Config, RuntimeFlags};
pub use error::CoreError;
pub use harness::{
    Access, Harness, HarnessBuilder, HarnessCore, HarnessError, Provider, Workspace,
};
#[cfg(feature = "mcp")]
pub use harness::{HttpHeader, McpAuthConfig, McpServer};
#[cfg(feature = "medulla")]
pub use medulla::{
    AbortResult, Medulla, MedullaStatus, Message, RosterWorker, SendResult, SessionCreated,
    SessionDetail, SessionSummary, WireEventEnvelope,
};

use std::sync::Arc;

use crate::core::runtime::CoreRuntime;

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
    /// [`CoreBuilder::build`](crate::core::runtime::CoreBuilder::build). The
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
    /// backend services. A [`HostKind::Library`](crate::core::types::HostKind::Library)
    /// runtime does not need an app session for caller-supplied inference.
    pub fn auth(&self) -> Auth<'_> {
        Auth(&self.rt)
    }

    /// Typed access to the agent harness — run a turn, get a reply.
    ///
    /// Requires the `inference` domain family at runtime; with it off the turn
    /// returns [`CoreError::Unavailable`], because the routed chat entry point
    /// is registered under that group. Note
    /// [`DomainSet::harness`](crate::core::runtime::DomainSet::harness) leaves
    /// `inference` **off** despite its name — use
    /// [`DomainSet::embedded`](crate::core::runtime::DomainSet::embedded), or
    /// set the field.
    pub fn agent(&self) -> Agent<'_> {
        Agent(&self.rt)
    }

    /// Typed access to the Medulla orchestration backend.
    ///
    /// Absent unless the `medulla` feature is on, so a host built without it
    /// fails to compile against this rather than meeting a runtime error.
    #[cfg(feature = "medulla")]
    pub fn medulla(&self) -> Medulla<'_> {
        Medulla(&self.rt)
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
