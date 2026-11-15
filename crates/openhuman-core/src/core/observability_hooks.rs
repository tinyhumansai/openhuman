//! Host-installed error and event classifiers.
//!
//! The core decides how to *act* on a failure (publish `SessionExpired`, demote
//! a Sentry report, drop a crash event) but not what a hosted backend's error
//! strings mean: the layer that produces those strings registers a classifier
//! here at boot. With nothing registered the core falls back to its own
//! generic markers, byte for byte.
//!
//! - [`register_session_expiry_predicate`] extends [`is_session_expired_error`].
//! - [`register_expected_error_classifier`] extends
//!   [`super::expected_error_kind`] at the credential/billing step.
//! - [`register_crash_noise_filter`] (crash-reporting) adds a Sentry
//!   `before_send` noise predicate after the built-in chain.

use std::sync::Arc;

use super::ExpectedErrorKind;
use crate::core::slots::HookSlot;

/// Whether an error string means the backend session expired.
pub type SessionExpiryPredicate = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The [`ExpectedErrorKind`] a host assigns to an error string it produced,
/// or `None` when the string is not one of its own.
pub type ExpectedErrorClassifier = Arc<dyn Fn(&str) -> Option<ExpectedErrorKind> + Send + Sync>;

static SESSION_EXPIRY_PREDICATES: HookSlot<SessionExpiryPredicate> =
    HookSlot::new("observability::session_expiry_predicates");

static EXPECTED_ERROR_CLASSIFIERS: HookSlot<ExpectedErrorClassifier> =
    HookSlot::new("observability::expected_error_classifiers");

/// Registers `predicate` under `id` for the whole process.
pub fn register_session_expiry_predicate(id: impl Into<String>, predicate: SessionExpiryPredicate) {
    SESSION_EXPIRY_PREDICATES.register(id, predicate);
}

/// Removes the process-wide session-expiry predicate `id`.
pub fn unregister_session_expiry_predicate(id: &str) -> bool {
    SESSION_EXPIRY_PREDICATES.unregister(id)
}

/// Whether `msg` indicates an expired or missing backend session.
///
/// [`invoke_method`](crate::core::invoke::invoke_method) consults it before
/// publishing `SessionExpired`, and the JSON-RPC transport uses it to keep
/// those failures out of Sentry. The core's own markers
/// ([`super::is_session_expired_message`]: `Session expired`,
/// `SESSION_EXPIRED`, the local pre-flight guards) always count; a host adds
/// the wire shapes of its own backend through
/// [`register_session_expiry_predicate`].
pub fn is_session_expired_error(msg: &str) -> bool {
    super::is_session_expired_message(msg)
        || SESSION_EXPIRY_PREDICATES
            .entries()
            .iter()
            .any(|(_, predicate)| predicate(msg))
}

/// Registers `classifier` under `id` for the whole process.
pub fn register_expected_error_classifier(
    id: impl Into<String>,
    classifier: ExpectedErrorClassifier,
) {
    EXPECTED_ERROR_CLASSIFIERS.register(id, classifier);
}

/// Removes the process-wide expected-error classifier `id`.
pub fn unregister_expected_error_classifier(id: &str) -> bool {
    EXPECTED_ERROR_CLASSIFIERS.unregister(id)
}

/// The first registered classifier's answer for `message`.
pub(super) fn host_expected_error_kind(message: &str) -> Option<ExpectedErrorKind> {
    EXPECTED_ERROR_CLASSIFIERS
        .entries()
        .iter()
        .find_map(|(_, classifier)| classifier(message))
}

/// A named Sentry `before_send` noise predicate a host contributes.
#[cfg(feature = "crash-reporting")]
#[derive(Clone, Copy)]
pub struct CrashNoiseFilter {
    /// The grep-able tag in the drop log line.
    pub name: &'static str,
    /// Whether the event is known noise.
    pub matches: fn(&sentry::protocol::Event<'static>) -> bool,
}

#[cfg(feature = "crash-reporting")]
static CRASH_NOISE_FILTERS: HookSlot<CrashNoiseFilter> =
    HookSlot::new("observability::crash_noise_filters");

/// Registers `filter` for the whole process, keyed by its name.
#[cfg(feature = "crash-reporting")]
pub fn register_crash_noise_filter(filter: CrashNoiseFilter) {
    CRASH_NOISE_FILTERS.register(filter.name, filter);
}

/// Removes the process-wide crash-noise filter `name`.
#[cfg(feature = "crash-reporting")]
pub fn unregister_crash_noise_filter(name: &str) -> bool {
    CRASH_NOISE_FILTERS.unregister(name)
}

/// The name of the first registered filter that classifies `event` as noise.
#[cfg(feature = "crash-reporting")]
pub fn registered_crash_noise(event: &sentry::protocol::Event<'static>) -> Option<&'static str> {
    CRASH_NOISE_FILTERS
        .entries()
        .into_iter()
        .find(|(_, filter)| (filter.matches)(event))
        .map(|(_, filter)| filter.name)
}

#[cfg(test)]
#[path = "observability_hooks_tests.rs"]
mod tests;
