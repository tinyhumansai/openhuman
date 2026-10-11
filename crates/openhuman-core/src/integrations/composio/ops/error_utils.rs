//! Shared error helpers for the Composio op layer.

use crate::config::Config;

/// Toolkits that honour the `tags` query param on the backend tool-list endpoint.
/// Expand this list when a new toolkit gains tag support.
const TAG_QUERYABLE_TOOLKITS: &[&str] = &["github"];

/// Returns `true` when `tags` should be forwarded to the backend.
///
/// Tags are forwarded when no toolkit filter is active (`None` / empty slice)
/// or when at least one requested toolkit is in [`TAG_QUERYABLE_TOOLKITS`].
/// This is `pub(crate)` so `tools.rs` can reuse it without duplicating the list.
pub(crate) fn should_forward_tags(toolkits: Option<&[String]>) -> bool {
    match toolkits {
        None => true,
        Some(kits) => {
            kits.is_empty()
                || kits.iter().any(|k| {
                    TAG_QUERYABLE_TOOLKITS
                        .iter()
                        .any(|t| k.trim().eq_ignore_ascii_case(t))
                })
        }
    }
}

/// Result alias used by every `composio_*` op in this module.
pub(super) type OpResult<T> = std::result::Result<T, String>;

/// The answer every backend-mode member gives while there is no backend
/// credential (neither an app-session JWT nor a TinyHumans API key). Shared so the members agree on the wording, and because the wording is
/// load-bearing: `is_session_expired_message` recognises "no backend session
/// token", so the JSON-RPC boundary demotes this to an expected user-state
/// error instead of paging Sentry.
pub(crate) const COMPOSIO_NO_SESSION: &str =
    "composio unavailable: no backend session token. Sign in or set a TinyHumans API key.";

/// True when the user has selected Composio **direct** mode but has not yet
/// configured an API key (neither in the keychain nor `config.toml`).
///
/// This is a valid, user-controlled *setup* state — the user just flipped to
/// direct mode and is about to paste their key — NOT an operation failure.
/// Callers short-circuit to an empty result instead of letting the
/// mode-aware factory bail with "composio direct mode selected but no api key
/// is configured", which the desktop UI's 5 s poll would otherwise funnel to
/// Sentry on every tick (TAURI-RUST-R4).
///
/// Key presence MUST mirror the factory's own resolution in
/// [`resolve_composio_route`](super::super::client::resolve_composio_route) (`client/factory.rs`): a key counts if it is in the
/// keychain (`credentials::get_composio_api_key`) **or** in `config.toml`
/// (`config.composio.api_key`). Checking only the keychain would wrongly
/// short-circuit to an empty list for a user who configured their key via
/// `config.toml`, hiding their real connections.
pub(crate) fn direct_mode_without_key(config: &Config) -> OpResult<bool> {
    if config.composio.mode.trim() != crate::config::schema::COMPOSIO_MODE_DIRECT {
        return Ok(false);
    }
    let has_key = crate::security::credentials::get_composio_api_key(config)
        .map_err(|e| format!("[composio] get_composio_api_key failed: {e}"))?
        .or_else(|| {
            config
                .composio
                .api_key
                .as_ref()
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
        })
        .is_some();
    Ok(!has_key)
}

/// True when the user is in Composio **backend** mode (the default) but has no
/// backend credential yet — a fresh install before sign-in, or signed out, and
/// no TinyHumans API key stored.
///
/// Like [`direct_mode_without_key`], this is a valid *setup* state, not an
/// operation failure. Without a session there is no proxy route to give the
/// connector module: `modules::connectors::module_config` fails, `ensure_routed`
/// reconfigures the module with `{"route": "none"}`, and every routed member
/// answers "this module was loaded without a connector route" — which the op
/// layer then reported at error level (and to Sentry) on every periodic tick,
/// for a user who simply has not signed in (#6176). Callers answer with the quiet
/// [`COMPOSIO_NO_SESSION`] error instead — deliberately not an empty list,
/// unlike the direct-mode guard: no key means no tenant and truly no
/// connections, whereas no session only means the connections cannot be
/// reached yet, and `flows::validate_connection_refs` relies on `Err` meaning
/// "unknown" (fail open) rather than "none".
///
/// Credential presence MUST mirror the module route's own resolution:
/// `module_config` calls `integrations::build_client`, which takes the stored
/// API key first and the app-session JWT otherwise. Read those same sources —
/// not `build_client` itself, which logs per call and would recreate the noise
/// this guard removes.
///
/// A *failed* token lookup (store unreadable) deliberately returns `false`:
/// that is a real fault, and it must keep surfacing through the normal error
/// path rather than being read as "not signed in".
pub(crate) fn backend_mode_without_session(config: &Config) -> bool {
    let mode = config.composio.mode.trim();
    if !(mode.is_empty() || mode == crate::config::schema::COMPOSIO_MODE_BACKEND) {
        return false;
    }
    if crate::security::credentials::api_key::has_api_key(config) {
        return false;
    }
    match crate::security::credentials::jwt::get_session_token(config) {
        Ok(token) => token.as_deref().map(str::trim).is_none_or(|token| {
            token.is_empty()
                || crate::security::credentials::session_support::is_local_session_token(token)
        }),
        Err(error) => {
            tracing::warn!(
                "[composio] backend_mode_without_session: session lookup failed ({error}); \
                 not treating as signed out"
            );
            false
        }
    }
}

/// Report host-side Composio faults without provider content or credentials.
///
/// Module failures have already emitted a sanitized terminal event at the bus
/// boundary. Recognize their established wire/presentation forms rather than
/// modifying user-visible messages or reporting them again. Host failures keep
/// only the existing failure/status classification and a static message.
pub(crate) fn report_composio_op_error<E: std::fmt::Display + ?Sized>(operation: &str, err: &E) {
    let rendered = format!("{err:#}");
    let reported_member = tinyconnectors_bus::METHODS.iter().any(|member| {
        rendered
            .strip_prefix(member)
            .is_some_and(|tail| tail.starts_with(": ai.tinyhumans.tinybus.Error.Failed: "))
    });
    if reported_member || crate::core::observability::is_module_unavailable_message(&rendered) {
        // Bus failures are reported once, with sanitized module metadata.
        return;
    }
    let failure_tag = classify_composio_failure_tag(rendered.as_str());
    if failure_tag == "non_2xx" {
        if let Some(status) = extract_backend_returned_status(&rendered) {
            crate::core::observability::report_error_or_expected(
                "connector operation failed",
                "composio",
                operation,
                &[("failure", failure_tag), ("status", status.as_str())],
            );
            return;
        }
    }
    crate::core::observability::report_error_or_expected(
        "connector operation failed",
        "composio",
        operation,
        &[("failure", failure_tag)],
    );
}

/// Pick the `failure` tag for a composio op-layer error message based on
/// shape inspection. Transport-level reqwest chains tag as `"transport"`;
/// everything else (the dominant `Backend returned <status> …` shape) tags
/// as `"non_2xx"`.
///
/// Extracted so tests can pin the routing without a Sentry test client.
pub(crate) fn classify_composio_failure_tag(rendered: &str) -> &'static str {
    let lower = rendered.to_ascii_lowercase();
    let is_transport = crate::core::observability::contains_transient_transport_phrase(rendered)
        || lower.contains("error sending request");
    if is_transport {
        "transport"
    } else {
        "non_2xx"
    }
}

/// Extract the HTTP status code from a `Backend returned <status> ...`
/// rendering produced by the integrations layer. Returns `None` when no
/// numeric status follows the anchor phrase.
pub(crate) fn extract_backend_returned_status(rendered: &str) -> Option<String> {
    let lower = rendered.to_ascii_lowercase();
    let rest = lower.split_once("backend returned ")?.1;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    (!digits.is_empty()).then_some(digits)
}
