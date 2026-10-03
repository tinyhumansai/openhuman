//! How the `/rpc` handler reports a failed call.
//!
//! [`classify_failure`] is pure so the routing can be tested without a server.
//! Whether an error is a session expiry is core's call
//! ([`openhuman_core::core::session_expiry`]); this module only decides how
//! loudly the transport reports it.

use openhuman_core::core::session_expiry::is_session_expired_error;

/// Returns `true` when the error is the wallet's "not configured yet" message.
///
/// Wallet-backed RPCs return
/// [`openhuman_core::web3::wallet::WALLET_NOT_CONFIGURED_MESSAGE`] before
/// setup. That is expected user state, not an internal failure.
///
/// Matched against the shared wallet constant (exact equality) so a wording
/// change in the wallet layer fails the coupling test in `classify_tests.rs`
/// rather than silently letting the noise back into Sentry.
pub(super) fn is_wallet_not_configured_error(msg: &str) -> bool {
    msg == openhuman_core::web3::wallet::WALLET_NOT_CONFIGURED_MESSAGE
}

/// How the `/rpc` handler reports a failed call, in priority order.
///
/// Only [`FailureDisposition::Unexpected`] is an error-level Sentry event. The
/// rest are either expected boundary conditions or already reported elsewhere.
/// The JSON-RPC error returned to the caller is the same for every variant,
/// except that [`FailureDisposition::UsageProbeBackoff`] replaces the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FailureDisposition {
    /// The controller's structured envelope set `expected_user_state` (stale
    /// thread refs and similar). Domains that surface their own expected
    /// user-state errors skip Sentry here uniformly.
    ExpectedUserState,
    /// A wallet-backed RPC cannot run before wallet setup. This is expected
    /// user state, not an internal failure.
    WalletNotConfigured,
    /// Param-validation failures ("unknown param 'x' for ns.fn", "missing
    /// required param 'x'", "invalid params: …") are pure boundary
    /// mismatches: either the caller is a frontend on a different release than
    /// the running core (OPENHUMAN-TAURI-20: v0.53.22 UI shipped `api_key`
    /// before the matching schema input landed in #1467) or it is straight
    /// client-bug input. Sentry cannot help — we can neither retro-fix
    /// already-shipped installs nor learn anything from the noise.
    ///
    /// Logged structurally with the body redacted: these messages embed
    /// caller-supplied param names and, for the `invalid params: …` shape, can
    /// carry deserialized values.
    ParamValidation,
    /// Session-expired bubbles up as an "error" but is an expected boundary
    /// condition (the auth handler clears the local token and the UI
    /// re-auths). Its messages are a small set of fixed strings with no
    /// caller-supplied content, so the full text is safe to log.
    SessionExpired,
    /// A `/teams/me/usage` probe that the failure-backoff in `team::ops`
    /// short-circuited within its window — i.e. an already-reported repeat.
    /// The first failure of the streak already hit the backend and reported
    /// normally; demoting the repeats is the flood control GH #4153 asks for
    /// (backpressure, not silent drop).
    ///
    /// The internal demotion marker must never reach the RPC client as the
    /// error message (CodeRabbit on #4153), so the handler replaces it with
    /// [`USAGE_BACKOFF_CLIENT_MESSAGE`].
    UsageProbeBackoff,
    /// A downstream call (backend_api / integrations / provider) already
    /// demoted the underlying transient failure to a warn. Re-reporting at
    /// error level would re-create the Sentry noise the lower-layer demote was
    /// meant to avoid (#8Z, #93, #8W, #96).
    ///
    /// The message is upstream-derived (backend / provider response) and can
    /// carry URL fragments, query params, or provider error text that
    /// includes tokens, so it is logged only after `sanitize_api_error`.
    TransientDownstream,
    /// An unrecognised RPC method is a transport-boundary mismatch (infra
    /// probe traffic, or a client on a different release than the running
    /// core), not an actionable core defect (#3567).
    ///
    /// Known external probes (`probe == true`) never become real methods, so
    /// they are debug-only and never reach Sentry. Any other unknown method is
    /// still recorded for triage, at warn severity (captured, no page).
    UnknownMethod { probe: bool },
    /// Everything else: reported through
    /// `observability::report_error_or_expected`.
    Unexpected,
}

/// What the caller sees instead of the usage-probe backoff marker.
pub(super) const USAGE_BACKOFF_CLIENT_MESSAGE: &str =
    "Usage temporarily unavailable — the last fetch failed and is backing off; \
     it will refresh shortly.";

/// Classify a failed call's display message. `expected_user_state` is the
/// flag from the controller's structured envelope, when it emitted one.
///
/// The order is significant: the first matching rule wins, exactly as the
/// handler's `if`/`else` chain did before it was extracted.
pub(super) fn classify_failure(message: &str, expected_user_state: bool) -> FailureDisposition {
    use openhuman_core::core::observability;

    if expected_user_state {
        FailureDisposition::ExpectedUserState
    } else if is_wallet_not_configured_error(message) {
        FailureDisposition::WalletNotConfigured
    } else if openhuman_core::core::params::is_param_validation_error(message) {
        FailureDisposition::ParamValidation
    } else if is_session_expired_error(message) {
        FailureDisposition::SessionExpired
    } else if observability::is_suppressed_usage_probe_backoff(message) {
        FailureDisposition::UsageProbeBackoff
    } else if observability::is_transient_message_failure(message) {
        FailureDisposition::TransientDownstream
    } else if let Some(unknown_method) =
        openhuman_core::core::dispatch::unknown_method_name(message)
    {
        FailureDisposition::UnknownMethod {
            probe: openhuman_core::core::dispatch::is_known_probe_method(unknown_method),
        }
    } else {
        FailureDisposition::Unexpected
    }
}

#[cfg(test)]
#[path = "classify_tests.rs"]
mod tests;
