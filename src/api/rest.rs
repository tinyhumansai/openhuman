//! HTTP client for TinyHumans / AlphaHuman API routes (`/auth/...`, etc.).

use anyhow::{Context, Result};
use base64::Engine;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use tinyhumans_sdk::{Error as SdkError, TinyHumansClient};

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
    /// `GET /announcements/latest` returned 404. The announcements feature is
    /// a best-effort, cosmetic fetch (`app/src/services/announcementService.ts`:
    /// "a missing announcement is never worth surfacing an error for") — a 404
    /// here means "no announcement", not a code bug. Callers should degrade to
    /// `null` and skip the retry-as-error path. Targets `TAURI-RUST-HW0`
    /// (`backend_api`/`authed_json`) and `TAURI-RUST-KHX` (`rpc`/`invoke_method`
    /// re-wrap) — one failure reported at two layers, ~452 events / 19 users.
    #[error("no announcement available (404 on /announcements/latest)")]
    AnnouncementNotFound,
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

/// `true` when `path` is `/announcements/latest`, tolerant of an arbitrary
/// base-path prefix (e.g. `/api/v1/announcements/latest`) — same
/// prefix-tolerant reasoning as [`parse_message_path`] (OPENHUMAN-TAURI-R7):
/// a `BACKEND_URL` override with a path prefix must not cause this route's
/// 404 to silently fall through to the generic `report_error` path.
fn is_announcements_latest_path(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    matches!(segments.as_slice(), [.., "announcements", "latest"])
}

const CLIENT_VERSION_HEADER_MAX_LEN: usize = 64;

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
            let keys = crate::openhuman::util::truncate_at_byte_boundary(
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

fn sanitize_client_version(raw: &str) -> Option<String> {
    let sanitized: String = raw
        .trim()
        .chars()
        .filter(|c| matches!(c, '0'..='9' | 'A'..='Z' | 'a'..='z' | '.' | '_' | '+' | '-'))
        .take(CLIENT_VERSION_HEADER_MAX_LEN)
        .collect();

    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

fn build_backend_reqwest_client() -> Result<Client> {
    let mut default_headers = HeaderMap::new();
    if let Some(version) = sanitize_client_version(env!("CARGO_PKG_VERSION")) {
        default_headers.insert(
            HeaderName::from_static("x-core-version"),
            HeaderValue::from_str(&version).context("invalid x-core-version header value")?,
        );
    }
    // The Tauri shell sets `OPENHUMAN_TAURI_VERSION` to its own package version
    // before spawning the in-process core, so backend analytics can attribute
    // core-originated requests to the desktop shell build that hosts them.
    if let Ok(raw) = std::env::var("OPENHUMAN_TAURI_VERSION") {
        if let Some(version) = sanitize_client_version(&raw) {
            default_headers.insert(
                HeaderName::from_static("x-tauri-version"),
                HeaderValue::from_str(&version).context("invalid x-tauri-version header value")?,
            );
        }
    }

    // Platform-appropriate TLS backend: Windows → schannel (honors the OS
    // cert store, required for corporate TLS-inspection proxies); macOS /
    // Linux → rustls. See [`crate::openhuman::util::tls::tls_client_builder`].
    crate::openhuman::util::tls::tls_client_builder()
        .default_headers(default_headers)
        .http1_only()
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {e}"))
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

/// Alias for [`user_id_from_profile_payload`] for semantic clarity in auth flows.
pub fn user_id_from_auth_me_payload(payload: &Value) -> Option<String> {
    user_id_from_profile_payload(payload)
}

/// JSON body returned by the backend when an OAuth connection process is initiated.
#[derive(Debug, Clone, Deserialize)]
pub struct ConnectResponse {
    /// The URL to redirect the user to for OAuth authorization.
    pub oauth_url: String,
    /// The state parameter used to prevent CSRF and correlate the callback.
    pub state: String,
}

/// A summary of an active integration, as returned by the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationSummary {
    /// Unique identifier for the integration.
    pub id: String,
    /// The name of the integration provider (e.g., "google", "slack").
    pub provider: String,
    /// RFC3339 timestamp of when the integration was created.
    pub created_at: String,
}

/// Decrypted OAuth token payload for handing off tokens to a local service or skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationTokensHandoff {
    /// The OAuth access token.
    pub access_token: String,
    /// The optional OAuth refresh token.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// RFC3339 timestamp of when the access token expires.
    pub expires_at: String,
}

/// A client for interacting with the TinyHumans / AlphaHuman backend API.
#[derive(Clone)]
pub struct BackendOAuthClient {
    client: Client,
    base: Url,
    sdk: TinyHumansClient,
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
        let client = build_backend_reqwest_client()?;
        let sdk = TinyHumansClient::new(base.as_str()).with_http_client(client.clone());
        Ok(Self { client, base, sdk })
    }

    /// Borrow the underlying `reqwest::Client` for callers that need to
    /// drive a non-JSON request shape (e.g. `multipart/form-data` uploads
    /// for cloud STT) without re-implementing TLS/proxy plumbing.
    pub fn raw_client(&self) -> &Client {
        &self.client
    }

    /// Resolve a backend-relative path against the configured base URL.
    /// Mirrors what `authed_json` does internally so callers using
    /// `raw_client()` don't have to assemble URLs by hand.
    pub fn url_for(&self, path: &str) -> Result<Url> {
        self.base
            .join(path.trim_start_matches('/'))
            .with_context(|| format!("build URL for {path}"))
    }

    /// Returns the URL for initiating a login flow for a specific provider.
    pub fn login_url(&self, provider: &str) -> Result<Url> {
        let p = provider.trim().trim_matches('/');
        anyhow::ensure!(!p.is_empty(), "provider is required");
        self.base
            .join(&format!("auth/{p}/login"))
            .context("build login URL")
    }

    /// Initiates an OAuth connection flow for the current user and a specific provider.
    pub async fn connect(
        &self,
        provider: &str,
        bearer_jwt: &str,
        skill_id: Option<&str>,
        response_type: Option<&str>,
        encryption_mode: Option<&str>,
    ) -> Result<ConnectResponse> {
        let p = provider.trim().trim_matches('/');
        anyhow::ensure!(!p.is_empty(), "provider is required");
        let query = {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            if let Some(s) = skill_id.filter(|s| !s.is_empty()) {
                serializer.append_pair("skillId", s);
            }
            if let Some(r) = response_type.filter(|r| !r.is_empty()) {
                serializer.append_pair("responseType", r);
            }
            if let Some(e) = encryption_mode.filter(|e| !e.is_empty()) {
                serializer.append_pair("encryptionMode", e);
            }
            serializer.finish()
        };
        let path = if query.is_empty() {
            format!("auth/{p}/connect")
        } else {
            format!("auth/{p}/connect?{query}")
        };
        let value = self
            .authed_json(bearer_jwt, Method::GET, &path, None)
            .await
            .context("auth connect request")?;
        let oauth_url = value
            .get("oauthUrl")
            .or_else(|| value.get("oauth_url"))
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .map(str::to_owned)
            .context("missing oauthUrl in response")?;
        let state = value
            .get("state")
            .and_then(Value::as_str)
            .filter(|state| !state.is_empty())
            .map(str::to_owned)
            .context("missing state")?;
        Ok(ConnectResponse { oauth_url, state })
    }

    /// Fetches the current authenticated user profile using the provided JWT.
    pub async fn fetch_current_user(&self, bearer_jwt: &str) -> Result<Value> {
        self.authed_json(bearer_jwt, Method::GET, "auth/me", None)
            .await
    }

    /// Exchanges a one-time login token (e.g. from Telegram) for a long-lived JWT.
    pub async fn consume_login_token(&self, login_token: &str) -> Result<String> {
        let token = login_token.trim();
        anyhow::ensure!(!token.is_empty(), "login token is required");

        // Backend serves `POST /auth/login-token/consume` with the token in a JSON
        // body `{ token, audience? }` and returns `{ success, data: { jwt } }`
        // (see backend `routes/auth.ts`). The legacy
        // `telegram/login-tokens/{token}/consume` path-param route was removed, so
        // the old call 404'd and Telegram/OAuth-token login could never complete
        // (WIRING_GAPS_AUDIT C1/C2).
        let response = self
            .sdk
            .auth()
            .consume_login_token(&tinyhumans_sdk::api::types::LoginTokenRequest {
                token: token.to_string(),
            })
            .await
            .context("consume login token through TinyHumans SDK")?;
        let jwt = response
            .get("jwt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        anyhow::ensure!(!jwt.is_empty(), "consume login token response missing jwt");
        Ok(jwt)
    }

    /// Validates that the provided session token is still active and accepted.
    pub async fn validate_session_token(&self, bearer_jwt: &str) -> Result<()> {
        let _ = self.fetch_current_user(bearer_jwt).await?;
        Ok(())
    }

    /// Creates a short-lived link token for connecting a specific communication channel.
    pub async fn create_channel_link_token(
        &self,
        channel: &str,
        bearer_jwt: &str,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        let encoded_channel = urlencoding::encode(channel);

        self.authed_json(
            bearer_jwt,
            Method::POST,
            &format!("auth/channels/{encoded_channel}/link-token"),
            None,
        )
        .await
    }

    /// Generic authenticated JSON request helper for backend API routes.
    pub async fn authed_json(
        &self,
        bearer_jwt: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        // OpenHuman does not consume backend webhook APIs. Keep that boundary
        // local even though the shared SDK intentionally exposes the
        // user-owned `/webhooks/core` tunnel CRUD surface for other clients.
        // Platform-admin operations are independently rejected by the SDK's
        // generated route gate below.
        anyhow::ensure!(
            !path
                .split(['/', '?'])
                .any(|segment| segment.eq_ignore_ascii_case("webhooks")),
            "backend webhook routes are not exposed through OpenHuman"
        );
        let url = self.url_for(path)?;
        let sdk = self
            .sdk
            .clone()
            .with_token(Some(bearer_jwt.trim().to_string()));
        let response = sdk
            .raw()
            .send(method.clone(), path, &[], body.as_ref(), true)
            .await;
        let value = match response {
            Ok(value) => return parse_api_response_value(value),
            Err(SdkError::Http(e)) => {
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
            Err(SdkError::Status { status, body }) => (status, body),
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
                .context("SDK returned an invalid HTTP status")?;
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
                return Err(anyhow::Error::new(BackendApiError::Unauthorized {
                    method: method.as_str().to_string(),
                    path: url.path().to_string(),
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

                // 404 on `/announcements/latest` means "no announcement" for
                // this best-effort, cosmetic feature — not a code bug. Surface
                // a typed `BackendApiError::AnnouncementNotFound` so the caller
                // (`announcements::ops::get_latest_announcement`) can degrade to
                // `null` instead of propagating an error, without funneling the
                // 404 into `report_error`. Targets `TAURI-RUST-HW0` / `TAURI-RUST-KHX`.
                if method == Method::GET && is_announcements_latest_path(url.path()) {
                    tracing::info!(
                        domain = "backend_api",
                        operation = "authed_json",
                        "[backend_api] announcement-not-found 404 on {} {} — surfacing typed error",
                        method.as_str(),
                        url.path(),
                    );
                    return Err(anyhow::Error::new(BackendApiError::AnnouncementNotFound));
                }
            }

            // These are transient infrastructure errors (proxy/CDN/backend
            // temporarily unavailable). They are not code bugs and callers already
            // implement retry/disable logic, so skip Sentry to avoid noise.
            let is_transient_infra =
                crate::core::observability::is_transient_http_status_code(status_code);
            let is_budget_exhausted = status_code == 400
                && crate::openhuman::inference::provider::is_budget_exhausted_message(&text);
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

    /// Lists all active integrations for the current user.
    pub async fn list_integrations(&self, bearer_jwt: &str) -> Result<Vec<IntegrationSummary>> {
        let value = self
            .authed_json(bearer_jwt, Method::GET, "auth/integrations", None)
            .await?;
        let integrations = value
            .get("integrations")
            .cloned()
            .unwrap_or_else(|| value.clone());
        serde_json::from_value(integrations).context("parse integrations response")
    }

    /// Fetches the decrypted OAuth tokens for a specific integration.
    ///
    /// This is a one-time handoff process. The encryption key must match the
    /// one used by the backend to encrypt the tokens.
    pub async fn fetch_integration_tokens_handoff(
        &self,
        integration_id: &str,
        bearer_jwt: &str,
        encryption_key: &str,
    ) -> Result<IntegrationTokensHandoff> {
        let id = integration_id.trim();
        anyhow::ensure!(
            !id.is_empty() && id.len() == 24,
            "integrationId must be a 24-char hex id"
        );
        let body = serde_json::json!({ "key": encryption_key.trim() });
        let value = self
            .authed_json(
                bearer_jwt,
                Method::POST,
                &format!("auth/integrations/{id}/tokens"),
                Some(body),
            )
            .await
            .context("integration tokens handoff")?;
        let encrypted = value
            .get("encrypted")
            .and_then(Value::as_str)
            .context("integration tokens response missing encrypted payload")?;
        let plaintext = decrypt_handoff_blob(encrypted, encryption_key.trim())?;
        serde_json::from_str(&plaintext).context("parse decrypted token JSON")
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

    /// Revokes (deletes) an active integration.
    pub async fn revoke_integration(&self, integration_id: &str, bearer_jwt: &str) -> Result<()> {
        let id = integration_id.trim();
        anyhow::ensure!(!id.is_empty(), "integration id is required");
        self.authed_json(
            bearer_jwt,
            Method::DELETE,
            &format!("auth/integrations/{id}"),
            None,
        )
        .await?;
        Ok(())
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
