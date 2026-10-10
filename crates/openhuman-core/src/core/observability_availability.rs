//! Sentinels for "a component this call needs is not available", and their
//! classifiers.
//!
//! Each one marks a condition that is decided once — no backend transport in
//! this build, a rejected API key, a native module that failed to load — and
//! then handed back to every caller. Re-reporting it per call carries no new
//! signal, so [`super::expected_error_kind`] demotes all of them.

/// Sentinel prefix on the error string a backend-touching call returns when
/// the core has no [`BackendTransport`](crate::backend::transport::BackendTransport)
/// installed. `backend::client::flatten_authed_error` and the integrations client
/// build their message from this constant; [`is_backend_unavailable_message`]
/// classifies it as [`super::ExpectedErrorKind::BackendUnavailable`].
pub const BACKEND_UNAVAILABLE_PREFIX: &str = "BACKEND_UNAVAILABLE:";

/// Sentinel prefix on the error string a backend call returns when the backend
/// rejects the stored TinyHumans API key (`backend::client::flatten_authed_error`).
/// [`super::expected_error_kind`] demotes it: the fix is a new key, not a code
/// change.
pub const API_KEY_REJECTED_PREFIX: &str = "API_KEY_REJECTED:";

/// Whether `msg` carries the [`API_KEY_REJECTED_PREFIX`] sentinel anywhere in
/// its chain.
pub fn is_api_key_rejected_message(msg: &str) -> bool {
    msg.contains(API_KEY_REJECTED_PREFIX)
}

/// Whether `msg` is the backend-unavailable sentinel (see
/// [`BACKEND_UNAVAILABLE_PREFIX`]). Matched anywhere in the chain because
/// callers wrap it with `anyhow` context before it reaches the reporter.
pub fn is_backend_unavailable_message(msg: &str) -> bool {
    msg.contains(BACKEND_UNAVAILABLE_PREFIX)
}

/// Whether `msg` is a native module's cached load failure
/// ([`super::ExpectedErrorKind::ModuleUnavailable`]).
///
/// tinybus never unloads a library, so `modules::ops` caches a load failure for
/// the life of the process and returns it instantly to every later caller; only
/// a restart changes the outcome. The failure is reported **once**, at
/// resolution (`modules::ops::report_resolution_failure`, tagged with the
/// module id). Every per-call re-report after that — composio ops, the `/rpc`
/// boundary, memory calls — was a few hundred broken installs producing ~1M
/// events (TAURI-RUST-117K / -118J / -117Y / -113J / -113T / -113D / -113N /
/// -113Q / -113X).
///
/// Anchors, produced by module loading or the shared module client:
///
/// - `MODULE_CALL_REPORTED:` — the shared client already emitted a sanitized
///   report; product callers must not report its returned error again;
///
/// - [`crate::tools::status::MODULE_FAULT_MARKER`] — every terminal load
///   failure from `modules::ops` and tinybus' `load_first_admitted` carries it;
/// - `module '<id>' could not be loaded` — the load wording itself, for a
///   caller that rewrapped the reason without the marker;
/// - `the memory module failed to load` — the memory facade's rendering of the
///   same cached failure.
///
/// Connector presentation strings also identify already-reported bus failures;
/// they retain their established wording without an internal marker.
///
/// A bare `could not be loaded` is deliberately not enough: config, update
/// policy and workflows use it for failures that must keep paging.
pub fn is_module_unavailable_message(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    // Connector provider errors preserve their established product wording.
    // Recognize only Composio provenance; a generic Execute failure can belong
    // to another module whose adapter has not migrated to this reporting path.
    let connector_output = msg.starts_with("[composio:error:")
        || msg.starts_with("Composio v3 ")
        || msg.starts_with("Failed to decode Composio v3 ")
        || [
            tinyconnectors_bus::names::methods::LIST_CONNECTIONS_DIRECT,
            tinyconnectors_bus::names::methods::LIST_TOOLS_DIRECT,
        ]
        .iter()
        .any(|member| {
            msg.strip_prefix(member)
                .is_some_and(|tail| tail.starts_with(": ai.tinyhumans.tinybus.Error.Failed: "))
        })
        || (msg.starts_with("[composio] ")
            && msg.contains(": ai.tinyhumans.tinybus.Error.Failed: "));
    connector_output
        || msg.contains("MODULE_CALL_REPORTED:")
        || msg.contains(crate::tools::status::MODULE_FAULT_MARKER)
        || (lower.contains("module '") && lower.contains("could not be loaded"))
        || (lower.contains("module '")
            && lower.contains("is unavailable: modules are disabled in configuration"))
        || lower.contains("the memory module failed to load")
}

/// The demoted report for [`super::ExpectedErrorKind::ModuleUnavailable`]: warn, so
/// the breadcrumb survives and a sustained spike still shows in logs, but no
/// Sentry error event — the one event was sent at resolution.
pub(super) fn log_module_unavailable(domain: &str, operation: &str, _message: &str) {
    tracing::warn!(
        domain = domain,
        operation = operation,
        kind = "module_unavailable",
        "[observability] {domain}.{operation} skipped expected module-unavailable error \
         (reported once at resolution)"
    );
}
