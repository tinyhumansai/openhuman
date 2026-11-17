//! Error type for the embedded typed facade.
//!
//! Every [`super::Core`] method returns [`CoreError`]. The variants exist to let
//! an embedding host (a TUI, a CLI, a test) tell four genuinely
//! different situations apart without parsing strings:
//!
//! - [`CoreError::Domain`] — the domain rejected the call and said why, via the
//!   [`StructuredRpcError`] envelope. Carries the stable `data.kind`
//!   discriminator and `expected_user_state`.
//! - [`CoreError::Unavailable`] — the method is not in this build at all,
//!   because a Cargo feature or a [`DomainSet`](openhuman_core::core::runtime::DomainSet)
//!   flag removed its controller. This is a *build fact*, not a failure.
//! - [`CoreError::Rpc`] — the call failed with a plain (unstructured) message.
//! - [`CoreError::Encode`] / [`CoreError::Decode`] — the facade's own
//!   serde boundary broke, which always means a facade bug, never a user error.
//!
//! The `Unavailable` split is the load-bearing one. `DomainSet` gates a domain
//! by *not registering its controllers* (`crates/openhuman-core/src/core/all.rs`), so a gated method
//! comes back as `unknown method: …` — indistinguishable from a typo unless the
//! facade classifies it. A host that cannot make that distinction renders a
//! scary red error where it should simply hide a tab.

use openhuman_core::core::dispatch::UNKNOWN_METHOD_PREFIX;
use openhuman_core::core::StructuredRpcError;

/// Error returned by every typed facade call.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// The host cancelled the call and its provider future has stopped.
    #[error("{method}: cancelled")]
    Cancelled {
        /// Cancelled operation.
        method: &'static str,
    },
    /// The entire logical call exceeded its deadline, including repairs.
    #[error("{method}: deadline exceeded")]
    DeadlineExceeded {
        /// Expired operation.
        method: &'static str,
    },
    /// Requested JSON failed strict validation after bounded repair.
    #[error("{method}: {failure}")]
    StructuredOutput {
        /// Operation that refused the answer.
        method: &'static str,
        /// Safe accounting and classification metadata.
        failure: crate::structured::StructuredOutputFailure,
    },

    /// Model call refused before dispatch by a shared run or turn budget.
    #[error("{method}: {source}")]
    BudgetExceeded {
        /// Method that attempted the call.
        method: &'static str,
        /// Refusal including spend so far and outstanding reservations.
        source: crate::budget::BudgetExceeded,
    },
    /// Physical model dispatch refused by the host before provider invocation.
    #[error("{method}: {source}")]
    DispatchAdmission {
        /// Method that attempted the call.
        method: &'static str,
        /// Typed host policy refusal.
        #[source]
        source: crate::budget::DispatchRefusal,
        /// Usage from completed calls in this logical completion, if any.
        usage: Option<crate::complete::CompletionUsage>,
    },
    /// The host cancelled this turn through its cancellation handle.
    #[error("{method}: turn cancelled")]
    TurnCancelled {
        /// RPC method the turn was dispatching.
        method: &'static str,
    },
    /// The domain returned a structured error envelope.
    #[error("{method}: {message}")]
    Domain {
        /// RPC method that produced the error.
        method: &'static str,
        /// Human-readable message from the domain.
        message: String,
        /// Stable `data.kind` discriminator, when the domain supplied one.
        kind: Option<String>,
        /// Full typed payload from the envelope.
        data: Option<serde_json::Value>,
        /// When true this is an expected user-visible state (stale session,
        /// missing resource), not an internal fault. Hosts should render it as
        /// a notice; the RPC boundary already skips Sentry for these.
        expected_user_state: bool,
    },

    /// The method is not present in this build (Cargo feature or `DomainSet`).
    ///
    /// Hosts should treat this as "this capability was compiled/configured
    /// out" and degrade the surface, not report a failure.
    #[error("{method}: not available in this build")]
    Unavailable {
        /// RPC method that is absent.
        method: &'static str,
    },

    /// The call failed with an unstructured error string.
    #[error("{method}: {message}")]
    Rpc {
        /// RPC method that produced the error.
        method: &'static str,
        /// Raw error text from the controller.
        message: String,
    },

    /// The facade could not serialize the request parameters.
    #[error("{method}: failed to encode params")]
    Encode {
        /// RPC method whose params failed to serialize.
        method: &'static str,
        /// Underlying serde error.
        #[source]
        source: serde_json::Error,
    },

    /// The facade could not deserialize the response into the expected type.
    ///
    /// This means the facade's typed struct and the domain's actual return
    /// shape have drifted apart — always a facade bug to fix, never something
    /// a host can recover from at runtime.
    #[error("{method}: failed to decode result")]
    Decode {
        /// RPC method whose result failed to deserialize.
        method: &'static str,
        /// Underlying serde error.
        #[source]
        source: serde_json::Error,
    },

    /// The route would transmit a bearer credential over a non-TLS channel.
    ///
    /// A [`crate::Route`] that names an `http://` (or other non-HTTPS)
    /// endpoint while carrying an `api_key` is refused before any request is
    /// sent, so the credential can never ride cleartext on the wire.
    #[error("{method}: refusing to send a bearer credential over a non-HTTPS route ({endpoint})")]
    InsecureRoute {
        /// RPC method the route was attached to.
        method: &'static str,
        /// The sanitized endpoint (credentials stripped) that was refused.
        endpoint: String,
    },

    /// A per-turn route was supplied but would be discarded by the inference
    /// controller because one of its required halves is blank.
    #[error("{method}: routed inference requires a non-blank endpoint and api key")]
    InvalidRoute {
        /// RPC method the invalid route was attached to.
        method: &'static str,
    },

    /// The agent was removed from its runtime: a turn sent afterwards is
    /// refused, and one in flight when it was removed ends with this.
    #[error("{method}: agent {agent_id:?} was removed")]
    AgentRemoved {
        /// RPC method the turn was dispatching.
        method: &'static str,
        /// The removed agent.
        agent_id: String,
    },
}

impl CoreError {
    /// Classify a raw controller error string into a typed variant.
    ///
    /// Order matters: the structured envelope is checked first because a domain
    /// could legitimately produce a message that happens to start with the
    /// unknown-method prefix.
    pub(crate) fn from_rpc_string(method: &'static str, raw: String) -> Self {
        if let Some(structured) = StructuredRpcError::decode(&raw) {
            let kind = structured
                .data
                .as_ref()
                .and_then(|d| d.get("kind"))
                .and_then(|k| k.as_str())
                .map(str::to_owned);
            log::debug!(
                "[embed] domain_error method={method} kind={kind:?} expected_user_state={}",
                structured.expected_user_state
            );
            return CoreError::Domain {
                method,
                message: structured.message,
                kind,
                data: structured.data,
                expected_user_state: structured.expected_user_state,
            };
        }

        if raw.starts_with(UNKNOWN_METHOD_PREFIX) {
            log::debug!("[embed] unavailable method={method} (gated out of this build)");
            return CoreError::Unavailable { method };
        }

        log::debug!("[embed] rpc_error method={method} message={raw}");
        CoreError::Rpc {
            method,
            message: raw,
        }
    }

    /// The RPC method this error came from.
    pub fn method(&self) -> &'static str {
        match self {
            CoreError::Cancelled { method }
            | CoreError::DeadlineExceeded { method }
            | CoreError::StructuredOutput { method, .. }
            | CoreError::BudgetExceeded { method, .. }
            | CoreError::Domain { method, .. }
            | CoreError::Unavailable { method }
            | CoreError::Rpc { method, .. }
            | CoreError::Encode { method, .. }
            | CoreError::Decode { method, .. }
            | CoreError::InsecureRoute { method, .. }
            | CoreError::InvalidRoute { method }
            | CoreError::AgentRemoved { method, .. }
            | CoreError::DispatchAdmission { method, .. }
            | CoreError::TurnCancelled { method } => method,
        }
    }

    /// True when the method is absent from this build.
    ///
    /// Sugar for the common host branch: hide the surface rather than error.
    pub fn is_unavailable(&self) -> bool {
        matches!(self, CoreError::Unavailable { .. })
    }

    /// True for expected user-visible states that should render as a notice.
    pub fn is_expected_user_state(&self) -> bool {
        matches!(
            self,
            CoreError::BudgetExceeded { .. }
                | CoreError::DispatchAdmission { .. }
                | CoreError::Domain {
                    expected_user_state: true,
                    ..
                }
        )
    }

    /// The stable `data.kind` discriminator, when the domain supplied one.
    pub fn kind(&self) -> Option<&str> {
        match self {
            CoreError::Domain { kind, .. } => kind.as_deref(),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
