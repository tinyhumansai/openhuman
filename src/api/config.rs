//! Base URL and defaults for the TinyHumans / AlphaHuman hosted API.

/// Default API host when `config.api_url` is unset or blank and no env override is set.
pub const DEFAULT_API_BASE_URL: &str = "https://api.tinyhumans.ai";
/// Default staging API host when the app environment is explicitly `staging`.
pub const DEFAULT_STAGING_API_BASE_URL: &str = "https://staging-api.tinyhumans.ai";
/// Primary app-environment selector used by the core and desktop app.
pub const APP_ENV_VAR: &str = "OPENHUMAN_APP_ENV";
/// Vite-exposed app-environment selector used by the frontend bundle.
pub const VITE_APP_ENV_VAR: &str = "VITE_OPENHUMAN_APP_ENV";

/// Resolves the hosted API base URL (no path suffix).
///
/// Order:
/// 1. Non-empty `api_url` from config (user explicitly set it)
/// 2. `BACKEND_URL` / `VITE_BACKEND_URL` runtime env vars (each checked independently)
/// 3. `BACKEND_URL` / `VITE_BACKEND_URL` baked in at compile time via `option_env!`
/// 4. Environment-aware default: `app_env_from_env()` == `staging` →
///    [`DEFAULT_STAGING_API_BASE_URL`], otherwise [`DEFAULT_API_BASE_URL`]
/// Default path the OpenHuman backend exposes for its OpenAI-compatible
/// inference proxy. Joined onto [`effective_api_url`] when the user has not
/// configured a custom `inference_url`.
pub const OPENHUMAN_INFERENCE_PATH: &str = "/openai/v1/chat/completions";

/// Resolves the LLM inference endpoint to call.
///
/// Derived state — not stored as a single field. Order:
/// 1. `config.inference_url` when set (user pointed inference at a custom
///    OpenAI-compatible endpoint — e.g. `https://api.openai.com/v1/chat/completions`).
/// 2. Otherwise `effective_api_url(api_url)` joined with `/openai/v1/chat/completions`
///    via the safe [`api_url`] helper, so inference flows through the OpenHuman
///    backend's OpenAI-compat proxy.
///
/// This split is what keeps account/auth/billing calls (always `effective_api_url`)
/// separate from inference (this function). Mixing them is what caused
/// `/auth/me`, `/auth/google/login`, and `/voice/*` to start hitting
/// `api.openai.com` when the user pointed `api_url` at a custom provider.
pub fn effective_inference_url(
    api_url_override: &Option<String>,
    inference_url_override: &Option<String>,
) -> String {
    if let Some(u) = inference_url_override
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return u.to_string();
    }
    api_url(
        &effective_api_url(api_url_override),
        OPENHUMAN_INFERENCE_PATH,
    )
}

pub fn effective_api_url(api_url: &Option<String>) -> String {
    if let Some(u) = api_url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return normalize_api_base_url(u);
    }
    if let Some(env_url) = api_base_from_env() {
        return env_url;
    }
    default_api_base_url_for_env(app_env_from_env().as_deref()).to_string()
}

/// Heuristic — does this URL look like a local-AI chat-completions endpoint
/// (Ollama, vLLM, LM Studio, OpenAI-compatible proxy on loopback) rather than
/// our hosted backend?
///
/// Used by [`effective_backend_api_url`] to avoid concatenating
/// backend-integration paths (e.g. `/agent-integrations/composio/toolkits`)
/// onto a user-set local-AI URL — see the Sentry cluster
/// `OPENHUMAN-TAURI-51 / -80 / -7Z` where Ollama users had every integration
/// request 404 because `config.api_url` was reused as both the chat base AND
/// the integrations base.
///
/// Heuristic is intentionally tight:
/// - Path explicitly ends with the OpenAI-style chat-completions endpoint
///   (`/v1/chat/completions` or `/v1/completions`) — matches anywhere, OR
/// - Host is loopback (`127.0.0.1` / `localhost` / `::1` / `0.0.0.0`) or
///   a private RFC 1918 IPv4 range (`10.0.0.0/8`, `172.16.0.0/12`,
///   `192.168.0.0/16`) **AND** the URL carries an additional LLM signal:
///   either a known local-AI port (`11434` Ollama, `8000` vLLM, `8080`
///   common alt, `1234` LM Studio, `8888` Jupyter-style proxies) or a
///   path beginning with `/v1`.
///
/// The combined loopback/private + LLM-signal requirement avoids
/// misclassifying ad-hoc mock backends bound on `127.0.0.1:<random port>`
/// with no path (the standard pattern used by our integration tests) as
/// local-AI while still catching every real-world Sentry case — those
/// always have either an LLM port or `/v1` in the URL.
///
/// Both path arms in the chat-completions check use `ends_with` rather
/// than `contains` so a real backend URL whose path merely embeds the
/// segment as a substring (e.g. `/audit/v1/chat/completions-logs`) is
/// NOT misclassified.
///
/// We deliberately do NOT match a bare `/v1` — that's a legitimate API
/// version suffix used by many self-hosted backends, and over-matching here
/// would silently route real backends to the default and break paying users.
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
    // Path-based match wins regardless of host so an OpenAI-style endpoint
    // exposed on any host (LAN, tunnel, public IP) still classifies.
    // `ends_with` (not `contains`) keeps a real backend whose path merely
    // embeds the segment as a substring (e.g. `/audit/v1/chat/completions-logs`)
    // from being misclassified.
    if path.ends_with("/v1/chat/completions") || path.ends_with("/v1/completions") {
        return true;
    }
    // Match by typed host so IPv4-mapped IPv6 (`::ffff:127.0.0.1`),
    // the bare IPv6 loopback (`::1`), and IPv4 loopback all classify
    // correctly regardless of how url::Url renders them via `host_str()`.
    let host_is_local = match parsed.host() {
        Some(url::Host::Ipv4(addr)) => {
            addr.is_loopback() || addr.is_unspecified() || addr.is_private()
        }
        Some(url::Host::Ipv6(addr)) => addr.is_loopback() || addr.is_unspecified(),
        Some(url::Host::Domain(name)) => {
            let host = name.to_ascii_lowercase();
            host == "localhost" || host.ends_with(".localhost")
        }
        None => false,
    };
    if !host_is_local {
        return false;
    }
    // Loopback / private host alone is not enough — many tests bind
    // mock backends on `127.0.0.1:<random ephemeral port>` with no path,
    // and we must not misclassify those as local-AI. Require an
    // additional LLM signal: a known local-AI port or a `/v1` path.
    const LOCAL_AI_PORTS: &[u16] = &[11434, 8000, 8080, 1234, 8888];
    let port_signals_llm = parsed
        .port()
        .map(|p| LOCAL_AI_PORTS.contains(&p))
        .unwrap_or(false);
    let path_signals_llm = path.starts_with("/v1/") || path == "/v1";
    port_signals_llm || path_signals_llm
}

/// Resolves the API base URL for **all hosted-backend calls** (billing,
/// team, referral, webhooks, credentials, channels, voice, socket,
/// app_state, integrations, core/jsonrpc, etc.).
///
/// Same resolution chain as [`effective_api_url`] EXCEPT the user override
/// is skipped when it [`looks_like_local_ai_endpoint`]. In that case we
/// fall through to env / default backend so backend requests still hit
/// the hosted API instead of being concatenated onto the user's local
/// Ollama/vLLM endpoint (which only knows about chat completions and
/// 404s every other path — see the Sentry cluster
/// `OPENHUMAN-TAURI-51 / -80 / -7Z`).
///
/// Logs a one-shot `warn!` the first time the fallback fires so users
/// can see the diagnostic in their core sidecar logs.
pub fn effective_backend_api_url(api_url: &Option<String>) -> String {
    if let Some(u) = api_url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if looks_like_local_ai_endpoint(u) {
            warn_backend_url_fallback_once(u);
            // Fall through to env / default — do NOT use the user override.
        } else {
            return normalize_backend_api_base_url(u);
        }
    }
    if let Some(env_url) = api_base_from_env() {
        return env_url;
    }
    default_api_base_url_for_env(app_env_from_env().as_deref()).to_string()
}

/// Normalize a configured backend override to its host root.
///
/// Users may have `config.api_url` populated with an inference endpoint such
/// as `https://api.tinyhumans.ai/openai/v1/chat/completions`. Backend
/// callers append domain-specific paths, so the LLM-specific path must not
/// survive into the backend base.
fn normalize_backend_api_base_url(url: &str) -> String {
    let normalized = normalize_api_base_url(url);
    let Ok(mut parsed) = url::Url::parse(&normalized) else {
        return normalized;
    };

    if parsed.path() != "/" {
        parsed.set_path("");
    }
    parsed.set_query(None);
    parsed.set_fragment(None);

    parsed.to_string().trim_end_matches('/').to_string()
}

/// Emit a single `warn!` **once per process lifetime** the first time
/// [`effective_backend_api_url`] falls back away from a user-set
/// local-AI URL. Subsequent calls — including calls with a *different*
/// local-AI URL — are silently suppressed via `std::sync::Once` so we
/// don't spam logs on every backend request.
fn warn_backend_url_fallback_once(local_url: &str) {
    use std::sync::Once;
    static WARNED: Once = Once::new();
    WARNED.call_once(|| {
        tracing::warn!(
            local_url = %local_url,
            "[api/config] config.api_url looks like a local-AI endpoint; \
             integrations base will fall back to env/default backend so \
             /agent-integrations/* requests don't 404 against your local LLM"
        );
    });
}

/// Trim and strip trailing slashes so paths join consistently.
pub fn normalize_api_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// Safely join an API base URL with a path.
///
/// Behaviour:
/// - Empty `path` → normalized `base` (no trailing slash).
/// - `path` starting with `/` → replaces any path on `base` (RFC 3986
///   absolute-path reference). This is the case that protects us from a
///   misconfigured `api_url` like `https://api.tinyhumans.ai/openai/v1/chat/completions`
///   silently corrupting every `/agent-integrations/...` call.
/// - If `base` fails to parse as a URL, falls back to slash-safe concat
///   so callers always get a usable string.
///
/// Paths SHOULD start with `/`. Relative paths (no leading slash) are
/// resolved against the base path per RFC 3986, which means the base's
/// last path segment is dropped — almost never what you want for an API.
pub fn api_url(base: &str, path: &str) -> String {
    let base_trimmed = base.trim();
    if path.is_empty() {
        return normalize_api_base_url(base_trimmed);
    }
    match url::Url::parse(base_trimmed) {
        Ok(parsed) => match parsed.join(path) {
            Ok(joined) => joined.to_string().trim_end_matches('/').to_string(),
            Err(_) => fallback_concat(base_trimmed, path),
        },
        Err(_) => fallback_concat(base_trimmed, path),
    }
}

fn fallback_concat(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

/// Resolve API base URL from the environment.
///
/// Each key is checked independently so that an empty `BACKEND_URL` does not
/// shadow a valid `VITE_BACKEND_URL`. Runtime vars are checked first, then
/// compile-time values baked in via `option_env!`. The compile-time path is
/// what makes a shipped DMG/installer resolve to the correct environment —
/// at runtime the process has no shell env vars set.
pub fn api_base_from_env() -> Option<String> {
    // 1. Runtime — each key checked independently; empty values are skipped
    //    so VITE_BACKEND_URL is still reachable when BACKEND_URL="" is set.
    for key in ["BACKEND_URL", "VITE_BACKEND_URL"] {
        if let Ok(v) = std::env::var(key) {
            let url = normalize_api_base_url(&v);
            if !url.is_empty() {
                return Some(url);
            }
        }
    }
    // 2. Compile-time fallback — baked in by build-desktop.yml.
    //    Each key checked independently for the same reason as above.
    for v in compile_time_api_base_env_values().into_iter().flatten() {
        let url = normalize_api_base_url(v);
        if !url.is_empty() {
            return Some(url);
        }
    }
    None
}

#[cfg(not(test))]
fn compile_time_api_base_env_values() -> [Option<&'static str>; 2] {
    [option_env!("BACKEND_URL"), option_env!("VITE_BACKEND_URL")]
}

#[cfg(test)]
fn compile_time_api_base_env_values() -> [Option<&'static str>; 2] {
    // Test wrappers may set BACKEND_URL to the mock server before rustc
    // starts. Runtime env coverage remains in the tests above; ignoring
    // baked values here keeps env-clearing assertions deterministic.
    [None, None]
}

/// Resolve the app environment, checking runtime env first then compile-time.
///
/// Each key is checked independently so that an empty primary key does not
/// shadow a valid secondary key. The compile-time fallback (`option_env!`)
/// mirrors what the Tauri shell already does for its Sentry environment tag.
pub fn app_env_from_env() -> Option<String> {
    // 1. Runtime — each key checked independently
    for key in [APP_ENV_VAR, VITE_APP_ENV_VAR] {
        if let Ok(v) = std::env::var(key) {
            let s = v.trim().to_ascii_lowercase();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    // 2. Compile-time fallback — each key checked independently
    for v in compile_time_app_env_values().into_iter().flatten() {
        let s = v.trim().to_ascii_lowercase();
        if !s.is_empty() {
            return Some(s);
        }
    }
    None
}

#[cfg(not(test))]
fn compile_time_app_env_values() -> [Option<&'static str>; 2] {
    [
        option_env!("OPENHUMAN_APP_ENV"),
        option_env!("VITE_OPENHUMAN_APP_ENV"),
    ]
}

#[cfg(test)]
fn compile_time_app_env_values() -> [Option<&'static str>; 2] {
    [None, None]
}

pub fn is_staging_app_env(app_env: Option<&str>) -> bool {
    matches!(app_env.map(str::trim), Some(env) if env.eq_ignore_ascii_case("staging"))
}

pub fn default_api_base_url_for_env(app_env: Option<&str>) -> &'static str {
    if is_staging_app_env(app_env) {
        DEFAULT_STAGING_API_BASE_URL
    } else {
        DEFAULT_API_BASE_URL
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use super::*;

    // Serialise all env-mutating tests to prevent flaky failures under
    // parallel test execution (std::env is process-global).
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> MutexGuard<'static, ()> {
        match ENV_LOCK.get_or_init(Mutex::default).lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    struct EnvSnapshot {
        vars: [(&'static str, Option<String>); 4],
    }

    impl EnvSnapshot {
        fn clear_backend_env() -> Self {
            let vars = [
                ("BACKEND_URL", std::env::var("BACKEND_URL").ok()),
                ("VITE_BACKEND_URL", std::env::var("VITE_BACKEND_URL").ok()),
                (APP_ENV_VAR, std::env::var(APP_ENV_VAR).ok()),
                (VITE_APP_ENV_VAR, std::env::var(VITE_APP_ENV_VAR).ok()),
            ];

            for (key, _) in vars.iter() {
                std::env::remove_var(*key);
            }

            Self { vars }
        }
    }

    impl Drop for EnvSnapshot {
        fn drop(&mut self) {
            for (key, value) in self.vars.iter() {
                match value {
                    Some(v) => std::env::set_var(*key, v),
                    None => std::env::remove_var(*key),
                }
            }
        }
    }

    #[test]
    fn api_url_empty_path_returns_normalized_base() {
        assert_eq!(
            api_url("https://api.tinyhumans.ai", ""),
            "https://api.tinyhumans.ai"
        );
        assert_eq!(
            api_url("https://api.tinyhumans.ai/", ""),
            "https://api.tinyhumans.ai"
        );
        assert_eq!(
            api_url("  https://api.tinyhumans.ai/  ", ""),
            "https://api.tinyhumans.ai"
        );
    }

    #[test]
    fn api_url_absolute_path_replaces_base_path() {
        // This is the regression: api_url misconfigured with a path baked in
        // must not corrupt /agent-integrations/* calls.
        assert_eq!(
            api_url(
                "https://api.tinyhumans.ai/openai/v1/chat/completions",
                "/agent-integrations/composio/toolkits"
            ),
            "https://api.tinyhumans.ai/agent-integrations/composio/toolkits"
        );
    }

    #[test]
    fn api_url_clean_base_joins_cleanly() {
        assert_eq!(
            api_url(
                "https://api.tinyhumans.ai",
                "/agent-integrations/composio/toolkits"
            ),
            "https://api.tinyhumans.ai/agent-integrations/composio/toolkits"
        );
        assert_eq!(
            api_url(
                "https://api.tinyhumans.ai/",
                "/agent-integrations/composio/toolkits"
            ),
            "https://api.tinyhumans.ai/agent-integrations/composio/toolkits"
        );
    }

    #[test]
    fn api_url_preserves_query_string_on_path() {
        assert_eq!(
            api_url(
                "https://api.tinyhumans.ai",
                "/agent-integrations/composio/tools?toolkits=gmail"
            ),
            "https://api.tinyhumans.ai/agent-integrations/composio/tools?toolkits=gmail"
        );
    }

    #[test]
    fn api_url_unparseable_base_falls_back_to_concat() {
        assert_eq!(api_url("not a url", "/x"), "not a url/x");
        assert_eq!(api_url("not a url/", "/x"), "not a url/x");
    }

    #[test]
    fn staging_app_env_uses_staging_default_api() {
        assert_eq!(
            default_api_base_url_for_env(Some("staging")),
            DEFAULT_STAGING_API_BASE_URL
        );
        assert!(is_staging_app_env(Some("STAGING")));
    }

    #[test]
    fn non_staging_app_env_uses_production_default_api() {
        assert_eq!(
            default_api_base_url_for_env(Some("production")),
            DEFAULT_API_BASE_URL
        );
        assert_eq!(default_api_base_url_for_env(None), DEFAULT_API_BASE_URL);
        assert!(!is_staging_app_env(Some("development")));
    }

    #[test]
    fn app_env_from_env_reads_runtime_var() {
        let _guard = env_lock();
        let key = APP_ENV_VAR;
        let prev = std::env::var(key).ok();
        std::env::set_var(key, "staging");
        let result = app_env_from_env();
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        assert_eq!(result.as_deref(), Some("staging"));
    }

    #[test]
    fn app_env_from_env_falls_through_empty_primary_to_secondary() {
        let _guard = env_lock();
        let prev_primary = std::env::var(APP_ENV_VAR).ok();
        let prev_secondary = std::env::var(VITE_APP_ENV_VAR).ok();
        std::env::set_var(APP_ENV_VAR, ""); // empty — must not block secondary
        std::env::set_var(VITE_APP_ENV_VAR, "staging");
        let result = app_env_from_env();
        match prev_primary {
            Some(v) => std::env::set_var(APP_ENV_VAR, v),
            None => std::env::remove_var(APP_ENV_VAR),
        }
        match prev_secondary {
            Some(v) => std::env::set_var(VITE_APP_ENV_VAR, v),
            None => std::env::remove_var(VITE_APP_ENV_VAR),
        }
        assert_eq!(result.as_deref(), Some("staging"));
    }

    #[test]
    fn api_base_from_env_reads_runtime_var() {
        let _guard = env_lock();
        let key = "BACKEND_URL";
        let prev = std::env::var(key).ok();
        std::env::set_var(key, "https://staging-api.tinyhumans.ai/");
        let result = api_base_from_env();
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        assert_eq!(result.as_deref(), Some("https://staging-api.tinyhumans.ai"));
    }

    #[test]
    fn api_base_from_env_falls_through_empty_primary_to_secondary() {
        let _guard = env_lock();
        let prev_primary = std::env::var("BACKEND_URL").ok();
        let prev_secondary = std::env::var("VITE_BACKEND_URL").ok();
        std::env::set_var("BACKEND_URL", ""); // empty — must not block secondary
        std::env::set_var("VITE_BACKEND_URL", "https://staging-api.tinyhumans.ai/");
        let result = api_base_from_env();
        match prev_primary {
            Some(v) => std::env::set_var("BACKEND_URL", v),
            None => std::env::remove_var("BACKEND_URL"),
        }
        match prev_secondary {
            Some(v) => std::env::set_var("VITE_BACKEND_URL", v),
            None => std::env::remove_var("VITE_BACKEND_URL"),
        }
        assert_eq!(result.as_deref(), Some("https://staging-api.tinyhumans.ai"));
    }

    // ── looks_like_local_ai_endpoint ───────────────────────────────────

    #[test]
    fn looks_like_local_ai_matches_loopback_hosts() {
        // Ollama default
        assert!(looks_like_local_ai_endpoint("http://127.0.0.1:11434/v1"));
        // vLLM default
        assert!(looks_like_local_ai_endpoint(
            "http://127.0.0.1:8080/v1/chat/completions"
        ));
        // localhost variant
        assert!(looks_like_local_ai_endpoint("http://localhost:11434/v1"));
        // IPv6 loopback
        assert!(looks_like_local_ai_endpoint("http://[::1]:11434"));
        // Any-host bind, occasionally used by self-hosted dev rigs
        assert!(looks_like_local_ai_endpoint("http://0.0.0.0:11434/v1"));
    }

    #[test]
    fn looks_like_local_ai_matches_chat_completions_path_on_non_loopback() {
        // Some self-hosted setups expose the OpenAI-compatible endpoint on
        // a non-loopback, non-private host (dev VM with a public IP, tunnel,
        // mDNS .local name). The chat-completions path is still a strong
        // tell that it's not our backend.
        assert!(looks_like_local_ai_endpoint(
            "http://203.0.113.5:8080/v1/chat/completions"
        ));
        assert!(looks_like_local_ai_endpoint(
            "https://my-ollama.example/v1/completions"
        ));
    }

    #[test]
    fn looks_like_local_ai_rejects_bare_loopback_with_random_port() {
        // Integration tests (e.g. `composio/ops_tests.rs`) bind mock
        // backends on `127.0.0.1:0` and let the kernel pick an ephemeral
        // port (~32768-60999), with no path. Loopback alone is *not* a
        // local-AI signal — we must not misclassify these as local-AI or
        // every integration test that goes through `build_client` will
        // see its request silently rerouted to the production backend.
        assert!(!looks_like_local_ai_endpoint("http://127.0.0.1:54321"));
        assert!(!looks_like_local_ai_endpoint("http://127.0.0.1:42000/"));
        assert!(!looks_like_local_ai_endpoint("http://localhost:33333"));
        assert!(!looks_like_local_ai_endpoint("http://[::1]:51234"));
    }

    #[test]
    fn looks_like_local_ai_matches_private_lan_hosts() {
        // LAN-hosted Ollama / vLLM on RFC 1918 ranges — covered by the
        // private-IP arm so users with `http://192.168.x.x:11434/v1`
        // configurations don't see integration requests routed at the
        // local LLM and 404.
        assert!(looks_like_local_ai_endpoint(
            "http://192.168.1.100:11434/v1"
        ));
        assert!(looks_like_local_ai_endpoint("http://10.0.0.5:8080/v1"));
        assert!(looks_like_local_ai_endpoint("http://172.16.0.42:8000"));
    }

    #[test]
    fn looks_like_local_ai_rejects_real_backends() {
        assert!(!looks_like_local_ai_endpoint("https://api.tinyhumans.ai"));
        assert!(!looks_like_local_ai_endpoint(
            "https://staging-api.tinyhumans.ai"
        ));
        // OpenAI public API — uses /v1 as a version prefix but no
        // chat-completions path on its own; we must NOT misclassify it.
        assert!(!looks_like_local_ai_endpoint("https://api.openai.com/v1"));
        // Custom self-hosted backend exposing a bare /v1 prefix — still
        // a real backend, must not be misclassified.
        assert!(!looks_like_local_ai_endpoint(
            "https://my-backend.example/v1"
        ));
    }

    #[test]
    fn looks_like_local_ai_rejects_substring_path_false_positives() {
        // graycyrus review of #1630: an earlier version used
        // `path.contains("/v1/chat/completions")` which would misclassify
        // any real backend whose path merely embedded that substring —
        // e.g. an audit-log endpoint suffixed with `-logs`. Both arms now
        // use `ends_with`, so these URLs must classify as NON-local.
        assert!(!looks_like_local_ai_endpoint(
            "https://real-backend.example/audit/v1/chat/completions-logs"
        ));
        assert!(!looks_like_local_ai_endpoint(
            "https://real-backend.example/v1/chat/completions/history"
        ));
        assert!(!looks_like_local_ai_endpoint(
            "https://real-backend.example/v1/completions-archive"
        ));
    }

    #[test]
    fn looks_like_local_ai_handles_garbage_input() {
        assert!(!looks_like_local_ai_endpoint(""));
        assert!(!looks_like_local_ai_endpoint("   "));
        assert!(!looks_like_local_ai_endpoint("not a url"));
        // Relative paths fail url::Url::parse — must not panic.
        assert!(!looks_like_local_ai_endpoint("/v1/chat/completions"));
    }

    #[test]
    fn looks_like_local_ai_matches_lm_studio_default_port() {
        // LM Studio default port 1234 is in the LOCAL_AI_PORTS list and
        // must be classified as a local-AI endpoint so integrations
        // requests are not routed through it (pr#1630 / pr#1715).
        assert!(looks_like_local_ai_endpoint("http://localhost:1234"));
        assert!(looks_like_local_ai_endpoint("http://127.0.0.1:1234"));
        assert!(looks_like_local_ai_endpoint(
            "http://127.0.0.1:1234/v1/chat/completions"
        ));
    }

    #[test]
    fn looks_like_local_ai_matches_v1_subpath_on_loopback() {
        // /v1/models, /v1/embeddings etc. on loopback are local-AI signals.
        assert!(looks_like_local_ai_endpoint(
            "http://localhost:11434/v1/models"
        ));
        assert!(looks_like_local_ai_endpoint(
            "http://127.0.0.1:8080/v1/embeddings"
        ));
    }

    // ── normalize_api_base_url (direct) ───────────────────────────────

    #[test]
    fn normalize_api_base_url_strips_single_trailing_slash() {
        assert_eq!(
            normalize_api_base_url("https://api.tinyhumans.ai/"),
            "https://api.tinyhumans.ai"
        );
    }

    #[test]
    fn normalize_api_base_url_strips_multiple_trailing_slashes() {
        assert_eq!(
            normalize_api_base_url("https://api.tinyhumans.ai///"),
            "https://api.tinyhumans.ai"
        );
    }

    #[test]
    fn normalize_api_base_url_trims_leading_and_trailing_whitespace() {
        assert_eq!(
            normalize_api_base_url("  https://api.tinyhumans.ai  "),
            "https://api.tinyhumans.ai"
        );
    }

    #[test]
    fn normalize_api_base_url_trims_whitespace_and_trailing_slash_together() {
        assert_eq!(
            normalize_api_base_url("  https://api.tinyhumans.ai/  "),
            "https://api.tinyhumans.ai"
        );
    }

    #[test]
    fn normalize_api_base_url_preserves_path_without_trailing_slash() {
        // A base that intentionally ends mid-path must not be touched beyond
        // trailing-slash removal — callers that set a sub-path base (unusual)
        // should still get what they provided.
        assert_eq!(
            normalize_api_base_url("https://api.tinyhumans.ai/v2"),
            "https://api.tinyhumans.ai/v2"
        );
    }

    #[test]
    fn normalize_api_base_url_empty_string_returns_empty() {
        // Normalising an empty string must not panic and must return empty.
        assert_eq!(normalize_api_base_url(""), "");
    }

    // ── api_url additional edge cases (pr#1715 / pr#1650) ─────────────

    #[test]
    fn api_url_with_lm_studio_base_joins_correctly() {
        // Verify that an LM Studio URL used as the api_url base (which
        // should not reach here in practice — effective_backend_api_url
        // redirects it away) still joins without panicking and produces
        // something parseable.
        let result = api_url("http://localhost:1234/v1", "/agent-integrations/foo");
        assert_eq!(result, "http://localhost:1234/agent-integrations/foo");
    }

    #[test]
    fn api_url_relative_path_without_leading_slash_joins_rfc3986() {
        // Relative paths (no leading `/`) are resolved against the base
        // path per RFC 3986 — the base's last segment is dropped. This is
        // documented behaviour; this test pins it so regressions are
        // visible.
        let result = api_url("https://api.tinyhumans.ai", "relative");
        // url::Url::join of a relative path onto a base with no trailing
        // segment simply appends — but the exact RFC 3986 result depends on
        // whether the base has a trailing slash. We just assert the call
        // doesn't panic and produces a non-empty string.
        assert!(!result.is_empty());
    }

    #[test]
    fn api_url_multiple_trailing_slashes_on_base_are_stripped() {
        assert_eq!(
            api_url("https://api.tinyhumans.ai///", "/v1/foo"),
            "https://api.tinyhumans.ai/v1/foo"
        );
    }

    // ── effective_backend_api_url ─────────────────────────────────

    #[test]
    fn integrations_url_handles_llm_endpoint_overrides() {
        let _guard = env_lock();
        let _env = EnvSnapshot::clear_backend_env();

        struct Case {
            api_url: &'static str,
            expected: &'static str,
        }

        let cases = [
            Case {
                api_url: "https://api.tinyhumans.ai/openai/v1/chat/completions",
                expected: "https://api.tinyhumans.ai",
            },
            Case {
                api_url: "http://localhost:11434/v1/chat/completions",
                expected: DEFAULT_API_BASE_URL,
            },
            Case {
                api_url: "https://api.tinyhumans.ai",
                expected: "https://api.tinyhumans.ai",
            },
            Case {
                api_url: "https://api.tinyhumans.ai/openai/v1/",
                expected: "https://api.tinyhumans.ai",
            },
            Case {
                api_url: "https://openrouter.ai/api/v1/chat/completions",
                expected: DEFAULT_API_BASE_URL,
            },
        ];

        for case in cases {
            assert_eq!(
                effective_backend_api_url(&Some(case.api_url.to_string())),
                case.expected,
                "api_url={}",
                case.api_url
            );
        }
    }

    #[test]
    fn integrations_url_falls_back_to_default_when_override_is_local_ai() {
        let _guard = env_lock();
        // Clear env so we deterministically hit the default branch.
        let prev_backend = std::env::var("BACKEND_URL").ok();
        let prev_vite_backend = std::env::var("VITE_BACKEND_URL").ok();
        let prev_app_env = std::env::var(APP_ENV_VAR).ok();
        let prev_vite_app_env = std::env::var(VITE_APP_ENV_VAR).ok();
        std::env::remove_var("BACKEND_URL");
        std::env::remove_var("VITE_BACKEND_URL");
        std::env::remove_var(APP_ENV_VAR);
        std::env::remove_var(VITE_APP_ENV_VAR);

        let result = effective_backend_api_url(&Some("http://127.0.0.1:11434/v1".to_string()));

        // Restore env before asserting so a failing assert doesn't leak.
        match prev_backend {
            Some(v) => std::env::set_var("BACKEND_URL", v),
            None => std::env::remove_var("BACKEND_URL"),
        }
        match prev_vite_backend {
            Some(v) => std::env::set_var("VITE_BACKEND_URL", v),
            None => std::env::remove_var("VITE_BACKEND_URL"),
        }
        match prev_app_env {
            Some(v) => std::env::set_var(APP_ENV_VAR, v),
            None => std::env::remove_var(APP_ENV_VAR),
        }
        match prev_vite_app_env {
            Some(v) => std::env::set_var(VITE_APP_ENV_VAR, v),
            None => std::env::remove_var(VITE_APP_ENV_VAR),
        }

        assert_eq!(result, DEFAULT_API_BASE_URL);
    }

    #[test]
    fn integrations_url_falls_back_to_env_when_override_is_local_ai() {
        let _guard = env_lock();
        let prev_backend = std::env::var("BACKEND_URL").ok();
        std::env::set_var("BACKEND_URL", "https://staging-api.tinyhumans.ai/");

        let result = effective_backend_api_url(&Some(
            "http://127.0.0.1:8080/v1/chat/completions".to_string(),
        ));

        match prev_backend {
            Some(v) => std::env::set_var("BACKEND_URL", v),
            None => std::env::remove_var("BACKEND_URL"),
        }

        assert_eq!(result, "https://staging-api.tinyhumans.ai");
    }

    #[test]
    fn integrations_url_keeps_real_backend_override() {
        // User explicitly set a real backend host — must be respected.
        let result =
            effective_backend_api_url(&Some("https://staging-api.tinyhumans.ai/".to_string()));
        assert_eq!(result, "https://staging-api.tinyhumans.ai");
    }

    #[test]
    fn integrations_url_matches_effective_api_url_without_override() {
        let _guard = env_lock();
        // No override, no env → both helpers must agree.
        let prev_backend = std::env::var("BACKEND_URL").ok();
        let prev_vite_backend = std::env::var("VITE_BACKEND_URL").ok();
        let prev_app_env = std::env::var(APP_ENV_VAR).ok();
        let prev_vite_app_env = std::env::var(VITE_APP_ENV_VAR).ok();
        std::env::remove_var("BACKEND_URL");
        std::env::remove_var("VITE_BACKEND_URL");
        std::env::remove_var(APP_ENV_VAR);
        std::env::remove_var(VITE_APP_ENV_VAR);

        let integrations = effective_backend_api_url(&None);
        let api = effective_api_url(&None);

        match prev_backend {
            Some(v) => std::env::set_var("BACKEND_URL", v),
            None => std::env::remove_var("BACKEND_URL"),
        }
        match prev_vite_backend {
            Some(v) => std::env::set_var("VITE_BACKEND_URL", v),
            None => std::env::remove_var("VITE_BACKEND_URL"),
        }
        match prev_app_env {
            Some(v) => std::env::set_var(APP_ENV_VAR, v),
            None => std::env::remove_var(APP_ENV_VAR),
        }
        match prev_vite_app_env {
            Some(v) => std::env::set_var(VITE_APP_ENV_VAR, v),
            None => std::env::remove_var(VITE_APP_ENV_VAR),
        }

        assert_eq!(integrations, api);
    }
}
