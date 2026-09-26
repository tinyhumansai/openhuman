//! HTTP client for TinyHumans / AlphaHuman API routes (`/auth/...`, etc.).

use anyhow::{Context, Result};
use base64::Engine;
use reqwest::{Client, Method, Url};

use serde_json::{json, Value};
use std::sync::Arc;

use crate::api::transport::{
    resolve_backend_transport, BackendRequest, BackendTransport, BackendTransportError,
    TransportProfile,
};
use crate::security::credentials::session_support::BackendCredential;

/// Typed errors surfaced by `authed_json` for expected backend states that
/// callers should recover from in-flow rather than funnel into Sentry.
#[derive(Debug, thiserror::Error)]
pub enum BackendApiError {
    /// Edit / delete of a channel message returned 404. Happens when the
    /// user deletes the message on the provider side (Telegram, Discord,
    /// Slack, …) but our local `StreamingState` still has the id, or when
    /// the backend GC'd the relay row before we got around to editing it.
    /// Callers should clear stale state and skip the retry. Targets
    /// `OPENHUMAN-TAURI-2Y` (~454 events on `/channels/telegram/messages/<id>`).
    #[error("message not found on {provider}: {message_id}")]
    MessageNotFound {
        /// Channel provider segment (e.g. `"telegram"`, `"discord"`).
        provider: String,
        /// Provider-specific message id from the URL.
        message_id: String,
    },
    /// Backend rejected the bearer JWT with `401 Unauthorized`. This is an
    /// expected user-session state (token expired, revoked, rotated
    /// server-side) — not a code bug. Callers can route to a re-sign-in
    /// flow; the auth domain owns recovery. Targets `OPENHUMAN-TAURI-4K8`
    /// (12 events on `/openai/v1/audio/speech` mascot TTS, but the same
    /// shape fires on every authed endpoint once the session lapses).
    #[error("backend rejected session token on {method} {path}")]
    Unauthorized {
        /// HTTP method as a static string (`"GET"`, `"POST"`, …).
        method: String,
        /// Request path the 401 came back from (no query string).
        path: String,
    },
    /// Backend rejected a TinyHumans API key (`x-api-key`) with
    /// `401 Unauthorized` — a library-mode runtime's credential, not a user
    /// session. Must stay distinct from [`Self::Unauthorized`]:
    /// `flatten_authed_error` maps that variant onto the `SESSION_EXPIRED`
    /// sentinel, which `core/jsonrpc.rs` treats as "clear the app session and
    /// sign out". A rejected API key on a runtime that never had a session
    /// would otherwise trigger that same session-expiry recovery, clearing an
    /// app session that was never the problem and leaving the rejected key
    /// installed. Callers should surface this as a credential error instead.
    #[error("backend rejected api key on {method} {path}")]
    ApiKeyRejected {
        /// HTTP method as a static string (`"GET"`, `"POST"`, …).
        method: String,
        /// Request path the 401 came back from (no query string).
        path: String,
    },
    /// `PATCH /channels/<provider>/messages/<id>` returned 404 because the
    /// backend **implements no such route** — not because the message is gone.
    ///
    /// The backend's channel router serves `POST /:channel/messages` and
    /// `DELETE /:channel/messages/:messageId` only; the `PATCH` edit route was
    /// never added (the deployed OpenAPI contract has no entry for it, and it
    /// is absent from the SDK's generated public-route registry). Every edit
    /// therefore hits the unmatched-route 404, and always has (#5230).
    ///
    /// This must stay distinct from [`Self::MessageNotFound`]: that variant
    /// means "this specific message no longer exists", so its handlers
    /// correctly forget the message id. A missing *route* says nothing about
    /// the message — it is still there and we still own it, so callers must
    /// keep the id (to delete or finally edit it) and only disable the edit
    /// capability. Collapsing the two made the live "💭 Thinking:" bubble and
    /// the streaming draft leak into the chat un-updated and un-deleted.
    ///
    /// Note the backend's `DELETE` handler answers only 400/403/502 and never
    /// 404, so on the deployed contract a 404 on this path can *only* mean
    /// route absence today. `MessageNotFound` is retained for `DELETE` because
    /// the provider-side-deletion semantics are what its callers want and a
    /// future backend revision may start returning it.
    #[error(
        "channel message edit route not implemented by backend ({provider}, message {message_id})"
    )]
    ChannelEditUnsupported {
        /// Channel provider segment (e.g. `"telegram"`, `"discord"`).
        provider: String,
        /// Provider-specific message id from the URL.
        message_id: String,
    },
    /// No [`BackendTransport`] is installed in this process, so the request
    /// could not be sent at all. This is the steady state of a core built
    /// and run without `openhuman-tinyhumans` — an expected build condition,
    /// never a bug. `flatten_authed_error` maps it onto the
    /// `BACKEND_UNAVAILABLE:` sentinel that
    /// `core::observability` demotes.
    #[error("backend transport not available for {method} {path}")]
    BackendUnavailable {
        /// HTTP method as a static string (`"GET"`, `"POST"`, …).
        method: String,
        /// Request path that could not be sent (no query string).
        path: String,
    },
}

/// Flatten an `authed_json` error onto the JSON-RPC `String` channel.
///
/// `BackendApiError::Unauthorized` is an expected backend session-lapse 401
/// (token expired / revoked / rotated server-side), not a code bug — see the
/// variant docs above. Callers used to flatten it with `format!("{e:#}")` /
/// `e.to_string()`, producing `"backend rejected session token on {method}
/// {path}"`, which matches none of the JSON-RPC session-expiry classifiers
/// (`is_session_expired_error`, `is_session_expired_message`, the `before_send`
/// net), so every lapsed-session 401 leaked to Sentry — TAURI-RUST-8WY
/// (`/teams/me/usage`), TAURI-RUST-8WZ (`/payments/stripe/currentPlan`), and the
/// rest of the authed-endpoint family (#3297).
///
/// Mapping `Unauthorized` onto the existing `SESSION_EXPIRED` sentinel makes the
/// dispatcher (`core/jsonrpc.rs`) classify it as session expiry: it skips the
/// Sentry report AND publishes `DomainEvent::SessionExpired` so the auth domain
/// drives re-sign-in. This keys off the typed downcast — not the Display
/// wording — so it stays correct if the `#[error(...)]` text changes, consistent
/// with #2959's removal of brittle string-based suppression. Every other error
/// (including `MessageNotFound`) keeps its full `{e:#}` chain so genuine
/// failures still reach Sentry.
pub fn flatten_authed_error(err: anyhow::Error) -> String {
    match err.downcast_ref::<BackendApiError>() {
        Some(BackendApiError::Unauthorized { method, path }) => {
            format!("SESSION_EXPIRED: backend rejected session token on {method} {path}")
        }
        // Deliberately NOT the `SESSION_EXPIRED` sentinel: this runtime
        // authenticates with an API key, not a session, so there is no
        // session to expire and `core/jsonrpc.rs`'s `SessionExpired` publish
        // (clear the session, prompt re-sign-in) would be the wrong
        // recovery. See `BackendApiError::ApiKeyRejected`.
        Some(BackendApiError::ApiKeyRejected { method, path }) => {
            format!(
                "{} backend rejected api key on {method} {path}",
                crate::core::observability::API_KEY_REJECTED_PREFIX
            )
        }
        Some(BackendApiError::BackendUnavailable { method, path }) => {
            format!(
                "{}backend transport not available for {method} {path}",
                crate::core::observability::BACKEND_UNAVAILABLE_PREFIX
            )
        }
        _ => format!("{err:#}"),
    }
}

/// Whether a 404 body came from *no route matching* rather than from a handler
/// reporting a missing resource.
///
/// The backend registers no catch-all 404, so an unmatched route falls through
/// to Express's built-in `finalhandler`, which answers with an HTML page whose
/// body reads `Cannot PATCH /channels/…`. Every handler-level 404 answers with a
/// JSON envelope instead — `DELETE /channels/:channel/messages/:messageId`
/// already returns `{"success": false, "error": …}`, and a future `PATCH`
/// handler would mirror it.
///
/// So: parses as JSON ⇒ a handler answered ⇒ the message is missing, not the
/// route. Anything else (HTML, plain text, empty) ⇒ treat as route absence.
///
/// The asymmetry is deliberate. Misreading route absence as message absence only
/// costs one wasted edit attempt per message; misreading a message-missing 404 as
/// route absence disables progressive edits for the entire provider for the rest
/// of the process (#5230 review). Defaulting the ambiguous shapes to route
/// absence also preserves today's behaviour, where no backend implements the
/// route at all.
fn is_unmatched_route_404(body: &str) -> bool {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return true;
    }
    serde_json::from_str::<Value>(trimmed).is_err()
}

/// Extract `(provider, message_id)` from a backend channel path of the
/// shape `…/channels/<provider>/messages/<id>`. Returns `None` for paths
/// that do not contain this four-segment subsequence.
///
/// Handles both the canonical four-segment form and paths with an arbitrary
/// base-path prefix (e.g. `/api/v1/channels/telegram/messages/1103`) via a
/// sliding window so that `BACKEND_URL` variants with path prefixes do not
/// silently fall through to `report_error` (OPENHUMAN-TAURI-R7).
fn parse_message_path(path: &str) -> Option<(&str, &str)> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    // Fast path: exact four-segment canonical form /channels/<p>/messages/<id>
    if segments.len() == 4 && segments[0] == "channels" && segments[2] == "messages" {
        return Some((segments[1], segments[3]));
    }
    // Sliding window: handles base-path prefixes like /api/v1/channels/<p>/messages/<id>
    for window in segments.windows(4) {
        if window[0] == "channels" && window[2] == "messages" {
            return Some((window[1], window[3]));
        }
    }
    None
}

/// Max bytes of the `body_shape` key-name list echoed into the `authed_json`
/// report. Bounded so a body with pathologically many keys can't bloat the
/// event; truncation is UTF-8-safe.
const BACKEND_API_BODY_SHAPE_MAX_BYTES: usize = 120;

/// PII-safe classification of a non-2xx response body for telemetry.
///
/// `report_error`'s message is written to the core/Tauri daily logs BEFORE any
/// Sentry `before_send` scrubbing, and that scrubber only catches a few
/// secret-shaped patterns — so the raw body must never be echoed (a non-2xx body
/// can carry emails / profile JSON / OAuth errors / nonstandard token fields).
/// We emit only the SHAPE: for a JSON object, the count of top-level keys plus
/// the sorted subset that look like schema field names; otherwise a coarse
/// label. Even key NAMES are response-controlled (a foreign backend could return
/// `{"jo@example.com": 1}`), so only keys matching a conservative ASCII-identifier
/// shape are echoed — everything else is counted as `redacted` and never logged.
/// The surviving names are enough to identify which backend/gateway produced a
/// response — the `TAURI-RUST-8C` case (a 91-byte body matching no route this
/// backend emits), where our canonical envelope is `{success,error,errorCode}`
/// and a foreign gateway/proxy is not.
fn backend_api_body_shape(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "empty".to_string();
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(map)) => {
            let total = map.len();
            let mut safe: Vec<&str> = map
                .keys()
                .map(String::as_str)
                .filter(|k| is_schema_like_key(k))
                .collect();
            safe.sort_unstable();
            let redacted = total - safe.len();
            // `safe` keys are ASCII identifiers, so the join is ASCII and the
            // truncation can only ever land on a byte boundary — but route it
            // through the UTF-8-safe truncator regardless (defence-in-depth).
            let keys = crate::util::truncate_at_byte_boundary(
                &safe.join(","),
                BACKEND_API_BODY_SHAPE_MAX_BYTES,
            );
            format!("object(keys={total},safe=[{keys}],redacted={redacted})")
        }
        Ok(Value::Array(_)) => "array".to_string(),
        Ok(_) => "scalar".to_string(),
        Err(_) => "non_json".to_string(),
    }
}

/// A JSON key safe to echo into telemetry: a short ASCII identifier (the shape
/// of a schema field name). Anything else — non-ASCII, punctuation like `@`,
/// whitespace, or overlong — is treated as response-controlled data and excluded
/// so `body_shape` can never leak an email/UUID/free-text used as a key.
fn is_schema_like_key(key: &str) -> bool {
    const MAX_KEY_LEN: usize = 40;
    !key.is_empty()
        && key.len() <= MAX_KEY_LEN
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// Normalize the backend envelope while preserving OpenHuman's historical
/// response shape. In particular, `/auth/me` returns `{success,user}` rather
/// than `{success,data}`; SDK transport must not expose that envelope detail to
/// existing callers.
fn parse_api_response_value(value: Value) -> Result<Value> {
    let Some(object) = value.as_object() else {
        return Ok(value);
    };
    if let Some(user) = object.get("user").filter(|user| !user.is_null()) {
        return Ok(user.clone());
    }
    let Some(success) = object.get("success").and_then(Value::as_bool) else {
        return Ok(value);
    };
    if !success {
        let message = object
            .get("message")
            .or_else(|| object.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("request unsuccessful");
        anyhow::bail!("API request failed: {message}");
    }
    if let Some(data) = object.get("data").filter(|data| !data.is_null()) {
        return Ok(data.clone());
    }
    if let Some(user) = object.get("user").filter(|user| !user.is_null()) {
        return Ok(user.clone());
    }
    let mut unwrapped = object.clone();
    unwrapped.remove("success");
    Ok(Value::Object(unwrapped))
}

fn user_id_from_object(obj: &serde_json::Map<String, Value>) -> Option<String> {
    for key in ["id", "_id", "userId"] {
        if let Some(s) = obj.get(key).and_then(|x| x.as_str()) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

/// Best-effort extraction of a user ID from an authenticated profile payload.
///
/// This function handles various envelope formats, including raw user objects
/// or those nested under `data` or `user` keys.
pub fn user_id_from_profile_payload(payload: &Value) -> Option<String> {
    let obj = payload.as_object()?;
    if let Some(data) = obj.get("data").and_then(|v| v.as_object()) {
        return user_id_from_object(data).or_else(|| {
            data.get("user")
                .and_then(|u| u.as_object())
                .and_then(user_id_from_object)
        });
    }

    user_id_from_object(obj).or_else(|| {
        obj.get("user")
            .and_then(|u| u.as_object())
            .and_then(user_id_from_object)
    })
}

/// A client for interacting with the TinyHumans / AlphaHuman backend API.
///
/// Owns the *routes* and the error classification; the HTTP round-trip itself
/// rides the process [`BackendTransport`] (see [`crate::api::transport`]),
/// which is what carries TLS, timeouts, attribution headers and the
/// credential header shape. A core with no transport installed answers every
/// call with [`BackendApiError::BackendUnavailable`].
#[derive(Clone)]
pub struct BackendOAuthClient {
    base: Url,
}

impl BackendOAuthClient {
    /// Creates a new `BackendOAuthClient` with the given API base URL.
    ///
    /// Any path, query, or fragment in `api_base` is stripped so that
    /// `Url::join` always resolves root-relative REST paths correctly.
    /// This guards against callers who pass a full LLM completions URL
    /// (e.g. `https://host/v1/chat/completions`) instead of just the origin:
    /// without stripping, `join("teams/me/usage")` would produce the wrong
    /// path `/v1/chat/teams/me/usage` via RFC 3986 relative resolution.
    pub fn new(api_base: &str) -> Result<Self> {
        let mut base = Url::parse(api_base.trim()).context("Invalid API base URL")?;
        anyhow::ensure!(
            matches!(base.scheme(), "http" | "https") && base.host_str().is_some(),
            "API base URL must be an absolute http(s) URL with host"
        );
        base.set_path("");
        base.set_query(None);
        base.set_fragment(None);
        Ok(Self { base })
    }

    /// The backend origin this client resolves routes against.
    pub fn base_url(&self) -> &str {
        self.base.as_str()
    }

    /// The process backend transport, or [`BackendApiError::BackendUnavailable`]
    /// for `method path` when none is installed.
    fn transport(&self, method: &Method, path: &str) -> Result<Arc<dyn BackendTransport>> {
        resolve_backend_transport().map_err(|error| match error {
            BackendTransportError::Unavailable => {
                let route = self.url_for(path).map(|u| u.path().to_string());
                log::debug!(
                    "[backend-api] no backend transport installed; {} {} unavailable",
                    method.as_str(),
                    route.as_deref().unwrap_or(path)
                );
                anyhow::Error::new(BackendApiError::BackendUnavailable {
                    method: method.as_str().to_string(),
                    path: route.unwrap_or_else(|_| path.to_string()),
                })
            }
            other => anyhow::Error::new(other),
        })
    }

    /// The transport's `reqwest::Client` for callers that need to drive a
    /// non-JSON request shape (e.g. `multipart/form-data` uploads for cloud
    /// STT) without re-implementing TLS/proxy plumbing. Carries the
    /// attribution headers; the caller adds the credential.
    pub fn raw_client(&self) -> Result<Client> {
        Ok(resolve_backend_transport()
            .map_err(|error| match error {
                BackendTransportError::Unavailable => {
                    anyhow::Error::new(BackendApiError::BackendUnavailable {
                        method: "RAW".to_string(),
                        path: String::new(),
                    })
                }
                other => anyhow::Error::new(other),
            })?
            .http_client(TransportProfile::Api))
    }

    /// Resolve a backend-relative path against the configured base URL.
    /// Mirrors what `authed_json` does internally so callers using
    /// `raw_client()` don't have to assemble URLs by hand.
    pub fn url_for(&self, path: &str) -> Result<Url> {
        self.base
            .join(path.trim_start_matches('/'))
            .with_context(|| format!("build URL for {path}"))
    }

    /// Generic authenticated JSON request helper for backend API routes.
    ///
    /// `credential` accepts a [`BackendCredential`] (from
    /// `session_support::resolve_backend_credential`) or, for the many callers
    /// that still hold a bare session token string, a `&str` / `&String`,
    /// which is treated as a session JWT. The transport puts it on the wire
    /// the backend expects for its kind: a session JWT as `Authorization:
    /// Bearer`, an API key as `x-api-key` (see `security::credentials::api_key`).
    pub async fn authed_json(
        &self,
        credential: impl Into<BackendCredential>,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let credential = credential.into();
        let is_api_key = credential.is_api_key();
        let transport = self.transport(&method, path)?;
        let response = transport
            .send_json(BackendRequest {
                profile: TransportProfile::Api,
                base_url: self.base.as_str(),
                method: method.clone(),
                path,
                query: &[],
                body: body.as_ref(),
                credential: Some(&credential),
                unwrap_envelope: true,
            })
            .await;
        self.finish_authed_json(method, path, response, is_api_key)
    }

    fn finish_authed_json(
        &self,
        method: Method,
        path: &str,
        response: Result<Value, BackendTransportError>,
        is_api_key: bool,
    ) -> Result<Value> {
        let url = self.url_for(path)?;
        let value = match response {
            Ok(value) => return parse_api_response_value(value),
            Err(BackendTransportError::Unavailable) => {
                return Err(anyhow::Error::new(BackendApiError::BackendUnavailable {
                    method: method.as_str().to_string(),
                    path: url.path().to_string(),
                }));
            }
            Err(BackendTransportError::Http(e)) => {
                // Walk the error source chain so transient markers hidden in nested
                // causes (reqwest -> hyper -> rustls TLS EOF, etc.) still classify
                // correctly. The top-level `e.to_string()` often only carries the
                // outermost wrapper, e.g. "error sending request for url (...)".
                let mut error_message = e.to_string();
                let mut src: Option<&(dyn std::error::Error + 'static)> =
                    std::error::Error::source(&e);
                while let Some(s) = src {
                    error_message.push_str(" → ");
                    error_message.push_str(&s.to_string());
                    src = s.source();
                }
                if crate::core::observability::contains_transient_transport_phrase(&error_message) {
                    tracing::warn!(
                        domain = "backend_api",
                        operation = "authed_json",
                        method = method.as_str(),
                        path = url.path(),
                        failure = "transport",
                        error = %error_message,
                        "[backend_api] transient transport failure on {} {}: {}",
                        method.as_str(),
                        url.path(),
                        error_message,
                    );
                } else {
                    crate::core::observability::report_error(
                        error_message.as_str(),
                        "backend_api",
                        "authed_json",
                        &[
                            ("method", method.as_str()),
                            ("path", url.path()),
                            ("failure", "transport"),
                        ],
                    );
                }
                return Err(anyhow::Error::new(e).context(format!(
                    "backend request {} {}",
                    method.as_str(),
                    url.path()
                )));
            }
            Err(BackendTransportError::Status { status, body }) => (status, body),
            Err(error) => {
                return Err(anyhow::Error::new(error).context(format!(
                    "backend request {} {}",
                    method.as_str(),
                    url.path()
                )));
            }
        };
        {
            let (status_code, response_body) = value;
            let status = reqwest::StatusCode::from_u16(status_code)
                .context("backend transport returned an invalid HTTP status")?;
            let text = match response_body {
                Value::String(text) => text,
                other => serde_json::to_string(&other).unwrap_or_default(),
            };
            let status_str = status_code.to_string();

            // 401 on any authed backend endpoint is an expected user-session
            // state (token expired / revoked / rotated server-side), not a
            // code bug — every authed endpoint will see this once the session
            // lapses. Surface a typed `BackendApiError::Unauthorized` so the
            // auth domain can drive recovery, and skip `report_error` to
            // avoid Sentry noise. Targets `OPENHUMAN-TAURI-4K8` (mascot TTS
            // surfaced it first on `/openai/v1/audio/speech`, but the same
            // shape applies to every `authed_json` path).
            if status_code == 401 {
                tracing::info!(
                    domain = "backend_api",
                    operation = "authed_json",
                    method = method.as_str(),
                    path = url.path(),
                    status = status_code,
                    failure = "non_2xx",
                    "[backend_api] 401 on {} {} — session token rejected, surfacing typed error",
                    method.as_str(),
                    url.path(),
                );
                // The credential kind decides the *recovery*, not just the
                // wording: `flatten_authed_error` maps `Unauthorized` onto
                // the `SESSION_EXPIRED` sentinel that triggers session
                // sign-out, which is the wrong recovery for a rejected API
                // key (there is no session to expire) — see
                // `BackendApiError::ApiKeyRejected`.
                return Err(anyhow::Error::new(if is_api_key {
                    BackendApiError::ApiKeyRejected {
                        method: method.as_str().to_string(),
                        path: url.path().to_string(),
                    }
                } else {
                    BackendApiError::Unauthorized {
                        method: method.as_str().to_string(),
                        path: url.path().to_string(),
                    }
                }));
            }

            // 404 on `/channels/<provider>/messages/<id>` is an expected
            // state (user deleted the message provider-side, or backend
            // GC'd the relay row) — not a code bug. Surface a typed
            // `BackendApiError::MessageNotFound` so callers (`bus.rs`
            // streaming/thinking/delete/final paths) can clear stale
            // ids and skip retry, without funneling the 404 into
            // `report_error`. Targets `OPENHUMAN-TAURI-2Y` (~454 events).
            if status_code == 404 {
                let channel_message = parse_message_path(url.path());
                // A 404 on the *edit* route is normally route absence, not
                // message absence — today the backend implements no `PATCH
                // /channels/:channel/messages/:messageId` at all (#5230). Answer
                // with a distinct typed error so `bus.rs` keeps the message id
                // (it still owns that message and must be able to delete it)
                // and only disables the edit capability. Checked before the
                // `MessageNotFound` arm below, which would otherwise swallow it.
                //
                // `is_unmatched_route_404` is what keeps this honest once the
                // route *does* exist (staging, a custom backend, or after the
                // backend PR lands): a handler-level "that message is gone" 404
                // must stay a per-message `MessageNotFound`, because
                // `ChannelEditUnsupported` makes `bus.rs` call
                // `mark_channel_edits_unsupported` and disable progressive edits
                // for the whole provider for the rest of the process.
                if method == Method::PATCH
                    && (channel_message.is_some()
                        || (url.path().contains("/channels/") && url.path().contains("/messages/")))
                    && is_unmatched_route_404(&text)
                {
                    let (provider, message_id) = channel_message
                        .map(|(provider, id)| (provider.to_string(), id.to_string()))
                        .unwrap_or_else(|| ("unknown".to_string(), "unknown".to_string()));
                    tracing::warn!(
                        domain = "backend_api",
                        operation = "authed_json",
                        provider = provider,
                        message_id = message_id,
                        "[backend_api] channel-message edit 404 on {} {} — backend implements no \
                         edit route; surfacing ChannelEditUnsupported so callers degrade instead \
                         of forgetting the message id (#5230)",
                        method.as_str(),
                        url.path(),
                    );
                    return Err(anyhow::Error::new(
                        BackendApiError::ChannelEditUnsupported {
                            provider,
                            message_id,
                        },
                    ));
                }

                if let Some((provider, message_id)) = channel_message {
                    tracing::info!(
                        domain = "backend_api",
                        operation = "authed_json",
                        provider = provider,
                        message_id = message_id,
                        "[backend_api] message-not-found 404 on {} {} — surfacing typed error",
                        method.as_str(),
                        url.path(),
                    );
                    return Err(anyhow::Error::new(BackendApiError::MessageNotFound {
                        provider: provider.to_string(),
                        message_id: message_id.to_string(),
                    }));
                }
                // Defense-in-depth: DELETE 404s on any channel-message path that
                // parse_message_path could not parse (e.g. exotic URL variant with extra
                // segments). Still an expected backend state — suppress the Sentry event
                // without propagating a typed error. Targets OPENHUMAN-TAURI-R7.
                // PATCH is handled above and returns the typed
                // `ChannelEditUnsupported` for both the parsed and unparsed shapes.
                if method == Method::DELETE
                    && url.path().contains("/channels/")
                    && url.path().contains("/messages/")
                {
                    tracing::debug!(
                        domain = "backend_api",
                        operation = "authed_json",
                        "[backend_api] channel-message 404 on {} {} — path not matched by \
                         parse_message_path, suppressing Sentry (TAURI-R7 defense-in-depth)",
                        method.as_str(),
                        url.path(),
                    );
                    anyhow::bail!(
                        "channel message not found (404) on {} {}",
                        method.as_str(),
                        url.path(),
                    );
                }
            }

            // These are transient infrastructure errors (proxy/CDN/backend
            // temporarily unavailable). They are not code bugs and callers already
            // implement retry/disable logic, so skip Sentry to avoid noise.
            let is_transient_infra =
                crate::core::observability::is_transient_http_status_code(status_code);
            let is_budget_exhausted =
                status_code == 400 && crate::api::classify::is_budget_exhausted_message(&text);
            if is_budget_exhausted {
                tracing::info!(
                    method = method.as_str(),
                    path = url.path(),
                    status = status_code,
                    failure = "non_2xx",
                    kind = "budget",
                    "[backend_api] budget-exhausted 400 on {} {} — not reporting to Sentry",
                    method.as_str(),
                    url.path(),
                );
            } else if is_transient_infra {
                tracing::warn!(
                    domain = "backend_api",
                    operation = "authed_json",
                    method = method.as_str(),
                    path = url.path(),
                    status = status_code,
                    failure = "non_2xx",
                    "[backend_api] transient {status} on {} {} — not reporting to Sentry",
                    method.as_str(),
                    url.path(),
                );
            } else {
                // Enrich the report with the two fields triage needs to pin a
                // non-2xx's origin: the outbound `host` and a PII-safe `body_shape`
                // (top-level JSON key names only — never values; see
                // `backend_api_body_shape`). `report_error` previously logged only
                // `response_body_len`, leaving us blind when a client hits a
                // non-canonical backend (custom BACKEND_URL / proxy / foreign
                // host) — TAURI-RUST-8C: 12k `GET /teams/me/usage` 404s from one
                // user whose 91-byte body matched no route this backend emits,
                // un-diagnosable because neither host nor shape was captured.
                // `host_str()` carries no scheme/path/query/token. Telemetry only
                // — the error still propagates below (no suppression).
                let host = url.host_str().unwrap_or("");
                let body_shape = backend_api_body_shape(&text);
                crate::core::observability::report_error(
                    format!(
                        "{} {} failed ({status}); response_body_len={}; body_shape={}",
                        method.as_str(),
                        url.path(),
                        text.len(),
                        body_shape,
                    )
                    .as_str(),
                    "backend_api",
                    "authed_json",
                    &[
                        ("method", method.as_str()),
                        ("path", url.path()),
                        ("host", host),
                        ("status", status_str.as_str()),
                        ("failure", "non_2xx"),
                    ],
                );
            }
            anyhow::bail!(
                "{} {} failed ({status}): {text}",
                method.as_str(),
                url.path()
            );
        }
    }

    /// Fetches the client key share for a specific integration.
    ///
    /// This is a one-time handoff; the key is deleted from the backend's
    /// temporary storage (Redis) after retrieval.
    pub async fn fetch_client_key(&self, integration_id: &str, bearer_jwt: &str) -> Result<String> {
        let id = integration_id.trim();
        anyhow::ensure!(
            !id.is_empty() && id.len() == 24,
            "integrationId must be a 24-char hex id"
        );
        let value = self
            .authed_json(
                bearer_jwt,
                Method::POST,
                &format!("auth/integrations/{id}/client-key"),
                None,
            )
            .await
            .context("fetch client key")?;
        let client_key = value
            .get("clientKey")
            .and_then(|k| k.as_str())
            .context("missing clientKey in response")?;
        Ok(client_key.to_string())
    }

    /// Sends a message to a communication channel.
    pub async fn send_channel_message(
        &self,
        channel: &str,
        bearer_jwt: &str,
        message_body: Value,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        let encoded = urlencoding::encode(channel);
        self.authed_json(
            bearer_jwt,
            Method::POST,
            &format!("channels/{encoded}/messages"),
            Some(message_body),
        )
        .await
    }

    /// Signals "the agent is typing…" on a channel that supports it
    /// (Telegram's `sendChatAction`, Slack's typing event, …). The backend
    /// resolves the target chat from the channel integration metadata and
    /// is responsible for hitting the provider-native API.
    ///
    /// Telegram keeps the typing indicator alive for ~5 seconds per call,
    /// so callers should re-invoke every ~4 s for as long as the turn is
    /// in flight. Returns `Err` if the backend doesn't support typing for
    /// this channel — caller should swallow the error silently.
    pub async fn send_channel_typing(&self, channel: &str, bearer_jwt: &str) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        let encoded = urlencoding::encode(channel);
        self.authed_json(
            bearer_jwt,
            Method::POST,
            &format!("channels/{encoded}/typing"),
            Some(json!({})),
        )
        .await
    }

    /// Edits an existing channel message. Used by the progressive-edit
    /// streaming path (Telegram / Slack) to coalesce live deltas into a
    /// single evolving outbound message rather than spamming the chat
    /// with one bubble per token.
    ///
    /// `message_id` is the backend-returned id of the message that was
    /// first sent via [`Self::send_channel_message`]. Returns the
    /// updated message record, or an `Err` if the backend does not
    /// support editing for this channel (caller should fall back to
    /// atomic-final delivery).
    ///
    /// **The deployed backend does not implement this route yet** (#5230): its
    /// channel router has `POST /:channel/messages` and `DELETE
    /// /:channel/messages/:messageId` only, so every call currently returns the
    /// unmatched-route 404, which [`authed_json`](Self::authed_json) surfaces as
    /// [`BackendApiError::ChannelEditUnsupported`]. Callers must degrade rather
    /// than treat that as the message having been deleted. Keep this method: it
    /// starts working unchanged the moment the backend adds the route.
    pub async fn send_channel_edit(
        &self,
        channel: &str,
        message_id: &str,
        bearer_jwt: &str,
        edit_body: Value,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        anyhow::ensure!(!message_id.is_empty(), "message_id is required");
        let encoded_channel = urlencoding::encode(channel);
        let encoded_id = urlencoding::encode(message_id);
        self.authed_json(
            bearer_jwt,
            Method::PATCH,
            &format!("channels/{encoded_channel}/messages/{encoded_id}"),
            Some(edit_body),
        )
        .await
    }

    /// Deletes a message from a communication channel. Used to clean up
    /// ephemeral messages (e.g. thinking indicators) after the final
    /// response has been delivered.
    pub async fn send_channel_delete(
        &self,
        channel: &str,
        message_id: &str,
        bearer_jwt: &str,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        anyhow::ensure!(!message_id.is_empty(), "message_id is required");
        let encoded_channel = urlencoding::encode(channel);
        let encoded_id = urlencoding::encode(message_id);
        self.authed_json(
            bearer_jwt,
            Method::DELETE,
            &format!("channels/{encoded_channel}/messages/{encoded_id}"),
            None,
        )
        .await
    }

    /// Sends a reaction (e.g. emoji) to a message in a channel.
    pub async fn send_channel_reaction(
        &self,
        channel: &str,
        bearer_jwt: &str,
        reaction_body: Value,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        let encoded = urlencoding::encode(channel);
        self.authed_json(
            bearer_jwt,
            Method::POST,
            &format!("channels/{encoded}/reactions"),
            Some(reaction_body),
        )
        .await
    }

    /// Creates a new thread in a communication channel.
    pub async fn create_channel_thread(
        &self,
        channel: &str,
        bearer_jwt: &str,
        title: &str,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        anyhow::ensure!(!title.trim().is_empty(), "title is required");
        let encoded = urlencoding::encode(channel);
        let body = serde_json::json!({ "title": title.trim() });
        self.authed_json(
            bearer_jwt,
            Method::POST,
            &format!("channels/{encoded}/threads"),
            Some(body),
        )
        .await
    }

    /// Updates an existing thread (e.g., closing or reopening it).
    pub async fn update_channel_thread(
        &self,
        channel: &str,
        bearer_jwt: &str,
        thread_id: &str,
        action: &str,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        anyhow::ensure!(!thread_id.trim().is_empty(), "threadId is required");
        anyhow::ensure!(
            action == "close" || action == "reopen",
            "action must be 'close' or 'reopen'"
        );
        let encoded_channel = urlencoding::encode(channel);
        let encoded_thread = urlencoding::encode(thread_id.trim());
        let body = serde_json::json!({ "action": action });
        self.authed_json(
            bearer_jwt,
            Method::PATCH,
            &format!("channels/{encoded_channel}/threads/{encoded_thread}"),
            Some(body),
        )
        .await
    }

    /// Lists threads in a communication channel, optionally filtering by status.
    pub async fn list_channel_threads(
        &self,
        channel: &str,
        bearer_jwt: &str,
        active_filter: Option<bool>,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        let encoded = urlencoding::encode(channel);
        let mut path = format!("channels/{encoded}/threads");
        if let Some(active) = active_filter {
            path.push_str(if active {
                "?active=true"
            } else {
                "?active=false"
            });
        }
        self.authed_json(bearer_jwt, Method::GET, &path, None).await
    }
}

/// AES-256-GCM decrypt compatible with backend `encryptMessageFromString` (IV 16 + tag 16 + ciphertext, base64).
pub fn decrypt_handoff_blob(b64_ciphertext: &str, key_str: &str) -> Result<String> {
    let key = key_bytes_from_string(key_str)?;
    let combined = base64::engine::general_purpose::STANDARD
        .decode(b64_ciphertext.trim())
        .context("base64-decode encrypted payload")?;
    if combined.len() < 32 {
        anyhow::bail!("encrypted payload too short");
    }
    let iv = &combined[0..16];
    let tag = &combined[16..32];
    let ciphertext = &combined[32..];

    // aes-gcm expects ciphertext || tag
    let mut ct_with_tag = Vec::with_capacity(ciphertext.len() + tag.len());
    ct_with_tag.extend_from_slice(ciphertext);
    ct_with_tag.extend_from_slice(tag);

    use aes_gcm::aead::generic_array::typenum::U16;
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::aes::Aes256;
    use aes_gcm::AesGcm;
    type Aes256Gcm16 = AesGcm<Aes256, U16>;

    let cipher =
        Aes256Gcm16::new_from_slice(&key).map_err(|e| anyhow::anyhow!("invalid AES key: {e}"))?;
    let nonce = aes_gcm::aead::generic_array::GenericArray::from_slice(iv);
    let plain = cipher
        .decrypt(nonce, ct_with_tag.as_ref())
        .map_err(|e| anyhow::anyhow!("AES-GCM decrypt failed: {e}"))?;

    String::from_utf8(plain).context("handoff plaintext is not UTF-8")
}

/// Decode the shared encryption key into 32 raw AES bytes.
///
/// Accepts, in order of preference:
/// 1. base64url without padding — the current backend format (e.g.
///    a 43-char alphanumeric string using `-` / `_`). This must be tried
///    BEFORE standard base64 because `-`/`_` are invalid in the standard
///    alphabet and would fail cleanly, whereas a standard-base64 string
///    never contains `-`/`_` so base64url_no_pad will still decode it
///    correctly as long as there's no padding.
/// 2. base64url with padding.
/// 3. Standard base64 with padding (legacy backend format).
/// 4. Standard base64 without padding.
/// 5. A raw 32-byte ASCII key (len == 32, used as-is).
///
/// NOTE: the key is only decoded locally for AES-GCM key material in
/// `decrypt_handoff_blob`. The key sent back to the backend (in the
/// `{ key: ... }` POST body of `fetch_integration_tokens_handoff`) is the
/// original string — never re-encoded — so base64url keys stay base64url
/// on the wire.
fn key_bytes_from_string(key: &str) -> Result<Vec<u8>> {
    let trimmed = key.trim();

    // Raw 32-byte ASCII key
    if trimmed.len() == 32 && !trimmed.contains(['+', '/', '-', '_', '=']) {
        return Ok(trimmed.as_bytes().to_vec());
    }

    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};

    // `base64::Engine` has generic methods and therefore isn't
    // dyn-compatible, so we unroll the attempts instead of looping over
    // a slice of trait objects.
    macro_rules! try_decode {
        ($engine:expr) => {
            if let Ok(decoded) = $engine.decode(trimmed) {
                if decoded.len() == 32 {
                    return Ok(decoded);
                }
            }
        };
    }
    try_decode!(URL_SAFE_NO_PAD);
    try_decode!(URL_SAFE);
    try_decode!(STANDARD);
    try_decode!(STANDARD_NO_PAD);

    anyhow::bail!(
        "encryption key must decode to 32 raw bytes (raw, base64, or base64url accepted; got len={})",
        trimmed.len()
    );
}

#[cfg(test)]
#[path = "rest_tests.rs"]
mod key_bytes_from_string_tests;
