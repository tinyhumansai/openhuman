//! Error type for the embedded typed facade.
//!
//! Every [`super::Core`] method returns [`CoreError`]. The variants exist to let
//! an embedding host (the Medulla TUI, a CLI, a test) tell four genuinely
//! different situations apart without parsing strings:
//!
//! - [`CoreError::Domain`] — the domain rejected the call and said why, via the
//!   [`StructuredRpcError`] envelope. Carries the stable `data.kind`
//!   discriminator and `expected_user_state`.
//! - [`CoreError::Unavailable`] — the method is not in this build at all,
//!   because a Cargo feature or a [`DomainSet`](crate::core::runtime::DomainSet)
//!   flag removed its controller. This is a *build fact*, not a failure.
//! - [`CoreError::Rpc`] — the call failed with a plain (unstructured) message.
//! - [`CoreError::Encode`] / [`CoreError::Decode`] — the facade's own
//!   serde boundary broke, which always means a facade bug, never a user error.
//!
//! The `Unavailable` split is the load-bearing one. `DomainSet` gates a domain
//! by *not registering its controllers* (`src/core/all.rs`), so a gated method
//! comes back as `unknown method: …` — indistinguishable from a typo unless the
//! facade classifies it. A host that cannot make that distinction renders a
//! scary red error where it should simply hide a tab.

use crate::core::dispatch::UNKNOWN_METHOD_PREFIX;
use crate::rpc::StructuredRpcError;

/// Error returned by every typed facade call.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
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
            CoreError::Domain { method, .. }
            | CoreError::Unavailable { method }
            | CoreError::Rpc { method, .. }
            | CoreError::Encode { method, .. }
            | CoreError::Decode { method, .. } => method,
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
            CoreError::Domain {
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn structured_envelope_becomes_domain_error() {
        let raw = StructuredRpcError {
            message: "thread thread-9 not found".to_string(),
            data: Some(json!({ "kind": "ThreadNotFound", "thread_id": "thread-9" })),
            expected_user_state: true,
        }
        .encode();

        let err = CoreError::from_rpc_string("openhuman.threads_get", raw);

        assert!(matches!(err, CoreError::Domain { .. }));
        assert_eq!(err.kind(), Some("ThreadNotFound"));
        assert!(err.is_expected_user_state());
        assert!(!err.is_unavailable());
        assert_eq!(err.method(), "openhuman.threads_get");
    }

    #[test]
    fn unknown_method_becomes_unavailable_not_rpc() {
        // This is the gated-domain path: DomainSet removes the controller, so
        // dispatch reports unknown-method. Hosts must be able to tell this from
        // a real failure or they render an error where they should hide a tab.
        let raw = format!("{UNKNOWN_METHOD_PREFIX}openhuman.flows_list");
        let err = CoreError::from_rpc_string("openhuman.flows_list", raw);

        assert!(err.is_unavailable(), "gated method must map to Unavailable");
        assert!(!err.is_expected_user_state());
        assert_eq!(err.kind(), None);
    }

    #[test]
    fn plain_string_becomes_rpc_error() {
        let err = CoreError::from_rpc_string("openhuman.config_get_config", "disk on fire".into());

        assert!(matches!(err, CoreError::Rpc { .. }));
        assert!(!err.is_unavailable());
        assert!(!err.is_expected_user_state());
    }

    #[test]
    fn structured_envelope_wins_over_unknown_method_prefix() {
        // A domain may legitimately produce a message that starts with the
        // unknown-method prefix. The envelope is authoritative.
        let raw = StructuredRpcError {
            message: format!("{UNKNOWN_METHOD_PREFIX}some.thing"),
            data: Some(json!({ "kind": "BadRequest" })),
            expected_user_state: false,
        }
        .encode();

        let err = CoreError::from_rpc_string("openhuman.x_y", raw);

        assert!(matches!(err, CoreError::Domain { .. }));
        assert!(!err.is_unavailable());
        assert_eq!(err.kind(), Some("BadRequest"));
    }

    #[test]
    fn expected_user_state_defaults_false_when_absent_from_envelope() {
        let raw = StructuredRpcError {
            message: "nope".to_string(),
            data: None,
            expected_user_state: false,
        }
        .encode();

        let err = CoreError::from_rpc_string("openhuman.a_b", raw);
        assert!(!err.is_expected_user_state());
    }
}
