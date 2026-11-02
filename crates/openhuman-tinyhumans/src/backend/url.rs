//! # API URL resolution & classification
//!
//! This module is the **single source of truth** for every URL the app uses to
//! reach either:
//!
//! * the **hosted backend** (auth, billing, integrations, voice, sockets, …), or
//! * the **LLM inference endpoint** (OpenAI-compatible chat completions).
//!
//! ## Why two separate URL families?
//!
//! Users can point `config.api_url` at a local model runner (Ollama, vLLM,
//! LM Studio). Those servers only speak `/v1/chat/completions` and 404 on
//! every other path. Naïvely reusing a single base URL for both families
//! caused every `/auth/*`, `/agent-integrations/*`, and `/voice/*` request to
//! 404 against the local runner — see Sentry cluster `OPENHUMAN-TAURI-51/-80/-7Z`.
//!
//! The fix is the [`effective_backend_api_url`] / [`effective_inference_url`]
//! split:
//!
//! ```text
//!                    config.api_url
//!                         │
//!              ┌──────────┴──────────┐
//!              │ looks_like_local_ai │
//!              └──────────┬──────────┘
//!                yes      │      no
//!         ┌───────────────┼────────────────────┐
//!         ▼               ▼                    ▼
//!  env / default   backend calls OK    inference calls OK
//!  (backend only)
//! ```
//!
//! ## Resolution order (both families)
//!
//! 1. Non-empty `config.api_url` / `config.inference_url` (user override).
//! 2. `BACKEND_URL` / `VITE_BACKEND_URL` runtime env (each checked
//!    independently so an empty primary does not shadow a valid secondary).
//! 3. Same keys baked in at compile time via `option_env!` (makes a
//!    distributed binary resolve to the correct environment without a shell).
//! 4. Environment-aware default: `staging` env → [`DEFAULT_STAGING_API_BASE_URL`],
//!    otherwise [`DEFAULT_API_BASE_URL`].

pub use openhuman_embed::__host::config::app_env::{
    app_env_from_env, is_staging_app_env, APP_ENV_VAR, VITE_APP_ENV_VAR,
};
pub use openhuman_embed::__host::util::url::{
    host_is_local, join_url, normalize_api_base_url, normalize_backend_api_base_url,
};

// ─── Public constants ────────────────────────────────────────────────────────

/// Production hosted-API root. Used as the final fallback for non-staging
/// builds when no override is configured.
pub const DEFAULT_API_BASE_URL: &str = "https://api.tinyhumans.ai";

/// Staging hosted-API root. Activated when `OPENHUMAN_APP_ENV=staging` (or
/// the Vite equivalent) is set at runtime or baked in at compile time.
pub const DEFAULT_STAGING_API_BASE_URL: &str = "https://staging-api.tinyhumans.ai";

/// The path the hosted backend appends to its root to expose the
/// OpenAI-compatible inference proxy. Joined onto [`effective_api_url`] when
/// the user has not configured a dedicated `inference_url`.
///
/// Having this as a named constant (rather than a string literal scattered
/// across call-sites) means a backend path rename shows up as a single diff.
pub const OPENHUMAN_INFERENCE_PATH: &str = "/openai/v1/chat/completions";

/// Convert a control-plane HTTP(S) base to the Socket.IO WebSocket handshake URL.
pub fn websocket_url(http_or_https_base: &str) -> String {
    let Ok(mut url) = url::Url::parse(http_or_https_base) else {
        return http_or_https_base.to_string();
    };
    let scheme = match url.scheme() {
        "https" => "wss".to_owned(),
        "http" => "ws".to_owned(),
        other => other.to_owned(),
    };
    let _ = url.set_scheme(&scheme);
    url.set_path(&format!("{}/socket.io/", url.path().trim_end_matches('/')));
    url.set_query(Some("EIO=4&transport=websocket"));
    url.to_string()
}

/// Socket.IO credentials are restricted to TLS endpoints and loopback HTTP.
pub fn is_safe_socket_endpoint(endpoint: &str) -> bool {
    let Ok(mut url) = url::Url::parse(endpoint) else {
        return false;
    };
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let scheme = match url.scheme() {
        "wss" => "https",
        "ws" => "http",
        _ => return false,
    };
    if url.set_scheme(scheme).is_err() {
        return false;
    }
    crate::client::endpoint::is_safe_endpoint_for_managed_bearer(url.as_str())
}

// ─── Known local-AI ports ────────────────────────────────────────────────────

/// Well-known ports used by local model runners.
///
/// Used by [`looks_like_local_ai_endpoint`] as a secondary signal when the
/// URL's host is loopback / private but the path alone is not conclusive
/// (e.g. `http://localhost:11434` — no path, but clearly Ollama).
///
/// | Port  | Runner        |
/// |-------|---------------|
/// | 11434 | Ollama        |
/// | 8000  | vLLM          |
/// | 8080  | common alt    |
/// | 1234  | LM Studio     |
/// | 8888  | Jupyter proxy |
const LOCAL_AI_PORTS: &[u16] = &[11434, 8000, 8080, 1234, 8888];

// ─── Effective URL resolvers ─────────────────────────────────────────────────

/// Resolve the URL for **LLM inference calls** (chat completions only).
///
/// # Resolution order
///
/// 1. `inference_url_override` — user explicitly pointed inference at a
///    custom OpenAI-compatible endpoint (e.g. `https://api.openai.com/v1/chat/completions`
///    or a local Ollama). Used as-is; no path stripping.
/// 2. [`effective_api_url`]`(api_url_override)` + [`OPENHUMAN_INFERENCE_PATH`] —
///    inference proxied through the hosted backend.
///
/// # Why the split matters
///
/// Without a dedicated `inference_url`, every inference call flows through the
/// hosted backend's OpenAI-compat proxy. When the user *does* set
/// `inference_url`, backend calls still go to [`effective_backend_api_url`] —
/// so `/auth/*`, `/voice/*`, and `/agent-integrations/*` never accidentally
/// hit `api.openai.com` or a local runner.
pub fn effective_inference_url(
    api_url_override: &Option<String>,
    inference_url_override: &Option<String>,
) -> String {
    // Explicit inference override always wins — no normalization applied
    // because the user may intentionally include a full path.
    if let Some(u) = non_empty_str(inference_url_override) {
        return u.to_string();
    }

    join_url(
        &effective_api_url(api_url_override),
        OPENHUMAN_INFERENCE_PATH,
    )
}

/// Resolve the **chat/inference base URL** (used for inference routing only,
/// not for backend domain calls).
///
/// Prefer [`effective_backend_api_url`] for anything other than chat completions.
/// The two functions are intentionally separate — see the module-level doc.
pub fn effective_api_url(api_url: &Option<String>) -> String {
    if let Some(u) = non_empty_str(api_url) {
        return normalize_api_base_url(u);
    }

    api_base_from_env()
        .unwrap_or_else(|| default_api_base_url_for_env(app_env_from_env().as_deref()).to_string())
}

/// Resolve the API base URL for **all hosted-backend calls**:
/// auth, billing, team, referral, webhooks, credentials, channels,
/// voice, sockets, app-state, integrations, core/jsonrpc, …
///
/// # Key difference from [`effective_api_url`]
///
/// The user override is **skipped** when it [`looks_like_local_ai_endpoint`]
/// **and** does not [`looks_like_openhuman_backend_endpoint`]. In that case
/// the function falls through to the env / default chain so backend requests
/// still reach the hosted API.
///
/// A one-shot `warn!` is emitted the first time the fallback fires so the
/// diagnostic is visible in sidecar logs without spamming on every request.
///
/// # Sentry context
///
/// `OPENHUMAN-TAURI-51 / -80 / -7Z` — Ollama users saw every integration
/// request 404 because `config.api_url` (set to the Ollama endpoint) was also
/// used as the integrations base.
pub fn effective_backend_api_url(api_url: &Option<String>) -> String {
    if let Some(u) = non_empty_str(api_url) {
        let is_local_ai = looks_like_local_ai_endpoint(u);
        let is_inference_provider = looks_like_inference_provider_endpoint(u);
        let is_openhuman = looks_like_openhuman_backend_endpoint(u);
        // A public third-party inference host (openrouter.ai, api.openai.com, …)
        // set to its canonical base (`https://openrouter.ai/api/v1`) is neither
        // local-AI nor an OpenHuman backend, so without this check the override
        // would be used as the backend base and every domain call (team usage,
        // billing) would 400/404 against the inference host — TAURI-RUST-HW1
        // (4932 `GET /teams/me/usage` 400s from `openrouter.ai`). Cloud analogue
        // of the local-AI guard (OPENHUMAN-TAURI-51/-80/-7Z, Ollama).
        let presets = crate::hosted_config::provider_presets();
        let is_cloud_inference = presets
            .endpoint_host(u)
            .is_some_and(|host| presets.host_is_builtin_cloud_provider(&host));

        tracing::debug!(
            api_url = %redact_url_for_log(u),
            is_local_ai,
            is_inference_provider,
            is_cloud_inference,
            is_openhuman,
            "[tinyhumans::backend::url] evaluating backend api_url override"
        );

        // Let the override through only when it is NOT an inference endpoint
        // (local model runner OR remote managed provider), OR when it is one of
        // our own hosted backends (user deliberately set `api_url` to
        // `https://api.tinyhumans.ai/openai/v1/chat/completions`).
        //
        // `config.api_url` doubles as the BYO inference base (see
        // `effective_inference_url`), so a user who points it at a managed
        // provider (`openrouter.ai`, `api.openmodel.ai`, …) was silently
        // sending every CONTROL-PLANE call there too — `/teams/me/usage`,
        // `/teams/*`, billing, referral — which the provider answers with a
        // 400/404/500. That misroute is the bulk of the `/teams/me/usage`
        // non-2xx flood (TAURI-RUST-BSF / -8C / -HDS / -HW1 / -JJ5, GH #4153):
        // the `host` tag added in #4058 pinned the destinations to
        // `openrouter.ai` / `api.openmodel.ai`, not our backend. The local-AI
        // arm already covered Ollama (`OPENHUMAN-TAURI-51 / -80 / -7Z`); this
        // widens the same fallback to remote providers. Inference routing is
        // unaffected — it resolves through `effective_api_url`, which never
        // consults this guard.
        //
        // `is_cloud_inference` (builtin cloud-provider host check, #4286) is
        // kept as an additional signal: it and `is_inference_provider` use
        // different detectors (builtin-cloud list vs curated inference domains +
        // OpenAI `/v1` path), so either flagging the override as an inference
        // base is enough to skip it — strictly safer against control-plane
        // misroute.
        if (!is_local_ai && !is_inference_provider && !is_cloud_inference) || is_openhuman {
            let normalized = normalize_backend_api_base_url(u);
            tracing::trace!(
                api_url        = %redact_url_for_log(u),
                normalized_url = %redact_url_for_log(&normalized),
                "[tinyhumans::backend::url] using configured backend api_url override"
            );
            return normalized;
        }

        tracing::debug!(
            api_url = %redact_url_for_log(u),
            is_local_ai,
            is_inference_provider,
            is_cloud_inference,
            "[tinyhumans::backend::url] override classified as inference endpoint (managed provider or builtin cloud host) — falling back to backend default chain"
        );
        warn_backend_url_fallback_once(u);
    }

    // Env / compile-time / default fallback — strip any inference path that
    // may have slipped through a misconfigured `BACKEND_URL` (Sentry
    // `OPENHUMAN-TAURI-H6 / -HN`, issue #2075).
    api_base_from_env()
        .map(|u| normalize_backend_api_base_url(&u))
        .unwrap_or_else(|| default_api_base_url_for_env(app_env_from_env().as_deref()).to_string())
}

// ─── URL classification ──────────────────────────────────────────────────────

/// Returns `true` when the URL appears to be a local / self-hosted model
/// runner rather than the hosted OpenHuman backend.
///
/// The heuristic is **intentionally tight** to avoid misclassifying:
/// * ad-hoc mock backends used in integration tests
///   (`http://127.0.0.1:<ephemeral-port>` with no path), and
/// * real custom backends that happen to include `/v1` as an API-version prefix.
///
/// # Classification logic
///
/// ```text
/// ┌─ path ends with /v1/chat/completions  ─────────────────────────────► TRUE
/// │  or /v1/completions (any host)
/// │
/// └─ host is loopback / private IP / localhost
///    AND (port ∈ LOCAL_AI_PORTS  OR  path starts with /v1/)  ──────────► TRUE
///
/// everything else  ────────────────────────────────────────────────────► FALSE
/// ```
///
/// Both path checks use `ends_with` (not `contains`) so a real backend whose
/// path merely *embeds* the segment (e.g. `/audit/v1/chat/completions-logs`)
/// is not misclassified.
///
/// A bare `/v1` path (e.g. `https://api.openai.com/v1`) intentionally does
/// NOT match — it is a legitimate API-version suffix used by many real
/// backends, and over-matching here would silently reroute paying users.
pub fn looks_like_local_ai_endpoint(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return false;
    }

    let parsed = match url::Url::parse(trimmed) {
        Ok(u) => u,
        Err(_) => return false,
    };

    let path = parsed.path();

    // ── Signal 1: chat-completions path (wins regardless of host) ──────────
    // `ends_with` not `contains` — see function doc.
    if path.ends_with("/v1/chat/completions") || path.ends_with("/v1/completions") {
        return true;
    }

    // ── Signal 2: loopback / private host + secondary LLM signal ───────────
    if !host_is_local(&parsed) {
        return false;
    }

    // Loopback alone is not enough — integration-test mock servers bind on
    // `127.0.0.1:<ephemeral>` with no path. Require at least one LLM signal.
    let port_is_llm = parsed
        .port()
        .map(|p| LOCAL_AI_PORTS.contains(&p))
        .unwrap_or(false);

    // `/v1/` (with trailing slash) or exactly `/v1` — avoids matching a bare
    // root `/` which is indistinguishable from any plain HTTP server.
    let path_is_llm = path.starts_with("/v1/") || path == "/v1";

    port_is_llm || path_is_llm
}

/// Well-known managed inference-provider registrable domains. A `config.api_url`
/// pointed at one of these (or a subdomain) is a BYO chat/inference base — never
/// an OpenHuman control-plane backend — so backend calls must NOT route there.
///
/// Suffix-matched so `api.<provider>` / `<region>.<provider>` also classify.
/// Kept tight to genuinely managed inference hosts; an unknown custom backend
/// is still honored unless it carries the OpenAI-compatible `/v1` base shape
/// below. `tinyhumans.ai` is deliberately ABSENT — our own hosted backend is
/// recognised by [`looks_like_openhuman_backend_endpoint`] and must route.
const INFERENCE_PROVIDER_DOMAINS: &[&str] = &[
    "openrouter.ai",
    "openmodel.ai",
    "atlascloud.ai",
    "openai.com",
    "anthropic.com",
    "groq.com",
    "mistral.ai",
    "deepseek.com",
    "together.ai",
    "together.xyz",
    "perplexity.ai",
    "fireworks.ai",
    "deepinfra.com",
    "anyscale.com",
    "novita.ai",
    "hyperbolic.xyz",
    "x.ai",
    "googleapis.com",
    "cohere.ai",
    "cohere.com",
];

/// Returns `true` when the URL looks like a **remote managed inference
/// provider** base rather than the hosted OpenHuman backend.
///
/// Complements [`looks_like_local_ai_endpoint`] (which only catches *local*
/// model runners): together they let [`effective_backend_api_url`] fall back to
/// the canonical backend whenever `config.api_url` has been set to an inference
/// base instead of a control-plane base. See the misroute note in
/// `effective_backend_api_url` (GH #4153 / TAURI-RUST-BSF·8C·HDS·HW1·JJ5).
///
/// Two signals (either is sufficient):
/// 1. **Known provider host** — host equals or is a subdomain of a domain in
///    [`INFERENCE_PROVIDER_DOMAINS`].
/// 2. **OpenAI-compatible base path** — the path is exactly `/v1` or `/api/v1`
///    (trailing slash ignored). This is the canonical OpenAI-style base and is
///    never an OpenHuman control-plane base. A bare `/v1/chat/completions` is
///    already covered by [`looks_like_local_ai_endpoint`]'s path signal.
///
/// Our own hosted backend short-circuits to `false` so a user who set
/// `api_url` to `https://api.tinyhumans.ai/...` still reaches the backend.
pub fn looks_like_inference_provider_endpoint(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Our own hosted backend is a backend, never a "foreign" inference base.
    if looks_like_openhuman_backend_endpoint(trimmed) {
        return false;
    }

    let parsed = match url::Url::parse(trimmed) {
        Ok(u) => u,
        Err(_) => return false,
    };

    // ── Signal 1: known managed provider host (apex or subdomain) ──────────
    if let Some(host) = parsed.host_str() {
        let host = host.to_ascii_lowercase();
        if INFERENCE_PROVIDER_DOMAINS
            .iter()
            .any(|d| host == *d || host.ends_with(&format!(".{d}")))
        {
            return true;
        }
    }

    // ── Signal 2: OpenAI-compatible bare `/v1` or `/api/v1` base path ──────
    // Exact-match both arms (per the doc contract): a longer self-hosted path
    // that merely *ends* in `/api/v1` (e.g. `https://backend.internal/svc/api/v1`)
    // is a custom backend, not an inference base, and must keep routing.
    let path = parsed.path().trim_end_matches('/');
    if path == "/api/v1" || path == "/v1" {
        return true;
    }

    false
}

/// Returns `true` when the URL's host is one of the known OpenHuman backends.
///
/// Used in [`effective_backend_api_url`] to short-circuit the local-AI check:
/// a user who set `api_url` to `https://api.tinyhumans.ai/openai/v1/chat/completions`
/// must still reach the real backend (not fall back to the default chain).
fn looks_like_openhuman_backend_endpoint(url: &str) -> bool {
    let trimmed = url.trim();
    let redacted = redact_url_for_log(trimmed);

    let parsed = match url::Url::parse(trimmed) {
        Ok(p) => {
            tracing::trace!(
                api_url = %redacted,
                "[tinyhumans::backend::url] parsed api_url for OpenHuman backend classification"
            );
            p
        }
        Err(e) => {
            tracing::trace!(
                api_url = %redacted,
                error   = %e,
                "[tinyhumans::backend::url] api_url parse failed during OpenHuman backend classification"
            );
            return false;
        }
    };

    let Some(host) = parsed.host_str().map(str::to_ascii_lowercase) else {
        tracing::trace!(
            api_url = %redacted,
            "[tinyhumans::backend::url] api_url has no host — not classified as OpenHuman backend"
        );
        return false;
    };

    let is_openhuman = matches!(
        host.as_str(),
        "api.tinyhumans.ai" | "staging-api.tinyhumans.ai"
    );

    tracing::debug!(
        api_url = %redacted,
        host    = %host,
        is_openhuman,
        "[tinyhumans::backend::url] OpenHuman backend classification complete"
    );

    is_openhuman
}

// ─── Environment resolution ───────────────────────────────────────────────────

/// Resolve the hosted API base URL from the environment.
///
/// Checks `BACKEND_URL` then `VITE_BACKEND_URL` independently (runtime first,
/// then compile-time bakes). An empty string for the primary key does **not**
/// shadow a valid secondary key — this matters when a `.env` file sets
/// `BACKEND_URL=""` to disable the override while keeping `VITE_BACKEND_URL`
/// active for the renderer.
///
/// Returns `None` when neither key is set or both are empty.
pub fn api_base_from_env() -> Option<String> {
    // 1. Runtime — each key checked independently.
    for key in ["BACKEND_URL", "VITE_BACKEND_URL"] {
        if let Ok(v) = std::env::var(key) {
            let url = normalize_api_base_url(&v);
            if !url.is_empty() {
                return Some(url);
            }
        }
    }

    // 2. Compile-time fallback — baked by the CI pipeline into the binary.
    //    Allows a shipped DMG / installer to resolve the correct environment
    //    without any shell vars in the user's session.
    for v in compile_time_api_base_env_values().into_iter().flatten() {
        let url = normalize_api_base_url(v);
        if !url.is_empty() {
            return Some(url);
        }
    }

    None
}

/// Map an app environment string to its canonical API base URL constant.
pub fn default_api_base_url_for_env(app_env: Option<&str>) -> &'static str {
    if is_staging_app_env(app_env) {
        DEFAULT_STAGING_API_BASE_URL
    } else {
        DEFAULT_API_BASE_URL
    }
}

// ─── Compile-time env accessors ───────────────────────────────────────────────

/// Values baked in by the build pipeline.
///
/// Stubbed to `[None, None]` in tests so that clearing runtime env vars
/// produces fully deterministic results regardless of what the CI baked in.
#[cfg(not(test))]
fn compile_time_api_base_env_values() -> [Option<&'static str>; 2] {
    [option_env!("BACKEND_URL"), option_env!("VITE_BACKEND_URL")]
}

#[cfg(test)]
fn compile_time_api_base_env_values() -> [Option<&'static str>; 2] {
    [None, None]
}

// ─── Logging helpers ─────────────────────────────────────────────────────────

/// Redact username and password from a URL before writing it to a log.
///
/// Falls back to a scheme-prefixed parse for bare-host strings like
/// `localhost:1234` so those are still sanitised rather than returned verbatim.
use openhuman_embed::__host::util::redact_url_for_log;

/// Emit a single `warn!` log the **first time** the backend URL falls back
/// from a user-set local-AI endpoint. Uses `std::sync::Once` to suppress
/// subsequent emissions so the log is not spammed on every backend request.
fn warn_backend_url_fallback_once(local_url: &str) {
    use std::sync::Once;
    static WARNED: Once = Once::new();
    WARNED.call_once(|| {
        tracing::warn!(
            local_url = %redact_url_for_log(local_url),
            "[tinyhumans::backend::url] config.api_url looks like a local-AI endpoint; \
             integrations base will fall back to env/default backend so \
             /agent-integrations/* requests don't 404 against your local LLM"
        );
    });
}

// ─── Private utilities ───────────────────────────────────────────────────────

/// Extract a trimmed, non-empty string reference from an `Option<String>`.
///
/// Centralises the `as_deref().map(str::trim).filter(|s| !s.is_empty())`
/// pattern that was repeated throughout the original code.
#[inline]
fn non_empty_str(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Process-global mutex serialising every test in this crate that mutates the
/// backend env vars (`BACKEND_URL` / `VITE_BACKEND_URL` / app-env overrides).
/// `std::env` is process-global, so a module-local lock cannot prevent
/// cross-module races: this module's own tests and `core::cli_tests`
/// tests all mutate the same vars from parallel test threads, and under the
/// full-suite coverage lane that race reliably broke the
/// "unconfigured" assertions. Every env-mutating test must take THIS lock.
#[cfg(test)]
pub(crate) fn backend_env_test_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ─── Tests ───────────────────────────────────────────────────────────────────
#[cfg(test)]
#[path = "url_tests.rs"]
mod tests;
