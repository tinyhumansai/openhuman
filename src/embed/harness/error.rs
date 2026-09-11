//! Errors a harness can produce.
//!
//! Split from [`CoreError`] rather than folded into it because the two answer
//! different questions. `CoreError` is about a *call*: the domain rejected it,
//! the method is not in this build, the facade's serde broke. `HarnessError`
//! adds the failures that happen before any call exists — laying out a
//! workspace, copying skills, discovering that a second core is already running
//! in this process.
//!
//! [`HarnessError::Call`] carries `CoreError` through unchanged, so a host that
//! already matches on `Unavailable` or on a domain `kind` keeps doing so.

use crate::embed::CoreError;

/// Error from building or driving a [`Harness`](super::Harness).
#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    /// A turn (or other RPC) failed. See [`CoreError`] for the distinctions.
    #[error(transparent)]
    Call(#[from] CoreError),

    /// The core itself failed to initialize.
    #[error("failed to build the embedded core: {0:#}")]
    Build(#[source] anyhow::Error),

    /// A filesystem operation setting up the workspace failed.
    #[error("failed to {what}")]
    Workspace {
        /// What was being attempted, phrased to complete "failed to …".
        what: &'static str,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// A second [`Harness`](super::Harness) was built in this process.
    ///
    /// Not a limitation of the harness but of the core it wraps: the keyring
    /// master key, the RPC bearer, the global event bus and the `Once`-guarded
    /// domain subscribers are all process-scoped (see the pluggable-core plan's
    /// phase 3). Two harnesses would share those while believing they had
    /// separate workspaces, which corrupts state quietly rather than loudly.
    /// Failing here is the loud version.
    #[error(
        "an OpenHuman harness is already running in this process; \
         core state (keyring, event bus, domain subscribers) is process-scoped, \
         so a second one would share it. Reuse the existing harness."
    )]
    AlreadyRunning,

    /// A builder input could not be honoured.
    #[error("{0}")]
    Invalid(String),
}

impl HarnessError {
    /// True when this is a build fact — a capability compiled or configured out
    /// — rather than a failure. Hosts should hide the surface, not report it.
    ///
    /// Mirrors [`CoreError::Unavailable`] so a caller need not unwrap the
    /// [`Call`](Self::Call) variant to ask the question.
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Call(CoreError::Unavailable { .. }))
    }
}
