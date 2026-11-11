use std::sync::MutexGuard;

use super::*;

#[test]
fn socket_url_uses_the_control_plane_socketio_handshake_path() {
    assert_eq!(
        websocket_url("https://api.tinyhumans.ai/base/"),
        "wss://api.tinyhumans.ai/base/socket.io/?EIO=4&transport=websocket"
    );
}

#[test]
fn socket_endpoint_guard_allows_tls_and_loopback_only() {
    assert!(is_safe_socket_endpoint(
        "wss://api.tinyhumans.ai/socket.io/"
    ));
    assert!(is_safe_socket_endpoint("ws://127.0.0.1:9000/socket.io/"));
    assert!(!is_safe_socket_endpoint("ws://example.com/socket.io/"));
    assert!(!is_safe_socket_endpoint(
        "wss://user@example.com/socket.io/"
    ));
}

// ── Test infrastructure ───────────────────────────────────────────────────

/// Serialises all env-mutating tests via the crate-wide backend env lock
/// (see [`super::backend_env_test_lock`]) — module-local locks cannot
/// stop other modules' tests from racing on the same process globals.
fn env_lock() -> MutexGuard<'static, ()> {
    super::backend_env_test_lock()
}

/// RAII guard that captures the current values of the four backend env
/// vars, removes them, and restores them on drop — even if the test panics.
struct EnvSnapshot {
    vars: [(&'static str, Option<String>); 4],
}

impl EnvSnapshot {
    fn clear_backend_env() -> Self {
        let keys = [
            "BACKEND_URL",
            "VITE_BACKEND_URL",
            APP_ENV_VAR,
            VITE_APP_ENV_VAR,
        ];
        let vars = keys.map(|k| (k, std::env::var(k).ok()));
        for (k, _) in &vars {
            std::env::remove_var(k);
        }
        Self { vars }
    }
}

impl Drop for EnvSnapshot {
    fn drop(&mut self) {
        for (key, value) in &self.vars {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

/// The URL that should be used as the backend base when no config override
/// is present and the runtime env has been cleared for the test.
fn fallback_backend_base_for_current_build() -> String {
    api_base_from_env()
        .unwrap_or_else(|| default_api_base_url_for_env(app_env_from_env().as_deref()).to_string())
}

// ── api_url ───────────────────────────────────────────────────────────────

#[test]
fn api_url_empty_path_returns_normalized_base() {
    assert_eq!(
        join_url("https://api.tinyhumans.ai", ""),
        "https://api.tinyhumans.ai"
    );
    assert_eq!(
        join_url("https://api.tinyhumans.ai/", ""),
        "https://api.tinyhumans.ai"
    );
    assert_eq!(
        join_url("  https://api.tinyhumans.ai/  ", ""),
        "https://api.tinyhumans.ai"
    );
}

#[test]
fn api_url_absolute_path_replaces_base_path() {
    // Regression: a base with an inference path baked in must not corrupt
    // /agent-integrations/* calls.
    assert_eq!(
        join_url(
            "https://api.tinyhumans.ai/openai/v1/chat/completions",
            "/agent-integrations/composio/toolkits",
        ),
        "https://api.tinyhumans.ai/agent-integrations/composio/toolkits"
    );
}

#[test]
fn api_url_clean_base_joins_cleanly() {
    let expected = "https://api.tinyhumans.ai/agent-integrations/composio/toolkits";
    assert_eq!(
        join_url(
            "https://api.tinyhumans.ai",
            "/agent-integrations/composio/toolkits"
        ),
        expected
    );
    assert_eq!(
        join_url(
            "https://api.tinyhumans.ai/",
            "/agent-integrations/composio/toolkits"
        ),
        expected
    );
}

#[test]
fn api_url_preserves_query_string_on_path() {
    assert_eq!(
        join_url(
            "https://api.tinyhumans.ai",
            "/agent-integrations/composio/tools?toolkits=gmail"
        ),
        "https://api.tinyhumans.ai/agent-integrations/composio/tools?toolkits=gmail"
    );
}

#[test]
fn api_url_unparseable_base_falls_back_to_concat() {
    assert_eq!(join_url("not a url", "/x"), "not a url/x");
    assert_eq!(join_url("not a url/", "/x"), "not a url/x");
}

#[test]
fn api_url_with_lm_studio_base_joins_correctly() {
    // LM Studio URL must not reach effective_backend_api_url in practice
    // (it redirects), but api_url itself must not panic and the result
    // must use the correct host root.
    assert_eq!(
        join_url("http://localhost:1234/v1", "/agent-integrations/foo"),
        "http://localhost:1234/agent-integrations/foo"
    );
}

#[test]
fn api_url_multiple_trailing_slashes_on_base_are_stripped() {
    assert_eq!(
        join_url("https://api.tinyhumans.ai///", "/v1/foo"),
        "https://api.tinyhumans.ai/v1/foo"
    );
}

#[test]
fn api_url_relative_path_without_leading_slash_does_not_panic() {
    // Documented edge-case: relative paths are resolved RFC 3986-style
    // (last base segment dropped). The exact result depends on base
    // structure; we just pin the no-panic contract.
    assert!(!join_url("https://api.tinyhumans.ai", "relative").is_empty());
}

// ── normalize_api_base_url ────────────────────────────────────────────────

#[test]
fn normalize_strips_trailing_slashes_and_whitespace() {
    assert_eq!(
        normalize_api_base_url("https://api.tinyhumans.ai/"),
        "https://api.tinyhumans.ai"
    );
    assert_eq!(
        normalize_api_base_url("https://api.tinyhumans.ai///"),
        "https://api.tinyhumans.ai"
    );
    assert_eq!(
        normalize_api_base_url("  https://api.tinyhumans.ai  "),
        "https://api.tinyhumans.ai"
    );
    assert_eq!(
        normalize_api_base_url("  https://api.tinyhumans.ai/  "),
        "https://api.tinyhumans.ai"
    );
}

#[test]
fn normalize_preserves_mid_path() {
    assert_eq!(
        normalize_api_base_url("https://api.tinyhumans.ai/v2"),
        "https://api.tinyhumans.ai/v2"
    );
}

#[test]
fn normalize_empty_string_returns_empty() {
    assert_eq!(normalize_api_base_url(""), "");
}

// ── normalize_backend_api_base_url ────────────────────────────────────────

#[test]
fn normalize_backend_strips_inference_path() {
    assert_eq!(
        normalize_backend_api_base_url("https://api.tinyhumans.ai/openai/v1/chat/completions"),
        "https://api.tinyhumans.ai"
    );
}

#[test]
fn normalize_backend_handles_schemeless_input() {
    // Sentry OPENHUMAN-TAURI-H6 / issue #2075.
    assert_eq!(
        normalize_backend_api_base_url("api.tinyhumans.ai/openai/v1/chat/completions"),
        "https://api.tinyhumans.ai"
    );
}

#[test]
fn normalize_backend_passes_through_clean_root() {
    assert_eq!(
        normalize_backend_api_base_url("https://api.tinyhumans.ai/"),
        "https://api.tinyhumans.ai"
    );
}

#[test]
fn normalize_backend_empty_string_is_idempotent() {
    assert_eq!(normalize_backend_api_base_url(""), "");
}

// ── app / api env resolution ──────────────────────────────────────────────

#[test]
fn staging_env_resolves_to_staging_url() {
    assert_eq!(
        default_api_base_url_for_env(Some("staging")),
        DEFAULT_STAGING_API_BASE_URL
    );
    assert!(is_staging_app_env(Some("STAGING")));
}

#[test]
fn non_staging_env_resolves_to_production_url() {
    assert_eq!(
        default_api_base_url_for_env(Some("production")),
        DEFAULT_API_BASE_URL
    );
    assert_eq!(default_api_base_url_for_env(None), DEFAULT_API_BASE_URL);
    assert!(!is_staging_app_env(Some("development")));
}

#[test]
fn api_base_from_env_reads_runtime_var() {
    let _guard = env_lock();
    let prev = std::env::var("BACKEND_URL").ok();
    std::env::set_var("BACKEND_URL", "https://staging-api.tinyhumans.ai/");
    let result = api_base_from_env();
    match prev {
        Some(v) => std::env::set_var("BACKEND_URL", v),
        None => std::env::remove_var("BACKEND_URL"),
    }
    assert_eq!(result.as_deref(), Some("https://staging-api.tinyhumans.ai"));
}

#[test]
fn api_base_empty_primary_falls_through_to_secondary() {
    let _guard = env_lock();
    let prev_p = std::env::var("BACKEND_URL").ok();
    let prev_s = std::env::var("VITE_BACKEND_URL").ok();
    std::env::set_var("BACKEND_URL", "");
    std::env::set_var("VITE_BACKEND_URL", "https://staging-api.tinyhumans.ai/");
    let result = api_base_from_env();
    match prev_p {
        Some(v) => std::env::set_var("BACKEND_URL", v),
        None => std::env::remove_var("BACKEND_URL"),
    }
    match prev_s {
        Some(v) => std::env::set_var("VITE_BACKEND_URL", v),
        None => std::env::remove_var("VITE_BACKEND_URL"),
    }
    assert_eq!(result.as_deref(), Some("https://staging-api.tinyhumans.ai"));
}

// ── looks_like_local_ai_endpoint ─────────────────────────────────────────

#[test]
fn local_ai_matches_loopback_hosts() {
    assert!(looks_like_local_ai_endpoint("http://127.0.0.1:11434/v1"));
    assert!(looks_like_local_ai_endpoint(
        "http://127.0.0.1:8080/v1/chat/completions"
    ));
    assert!(looks_like_local_ai_endpoint("http://localhost:11434/v1"));
    assert!(looks_like_local_ai_endpoint("http://[::1]:11434"));
    assert!(looks_like_local_ai_endpoint("http://0.0.0.0:11434/v1"));
}

#[test]
fn local_ai_matches_chat_completions_path_on_any_host() {
    assert!(looks_like_local_ai_endpoint(
        "http://203.0.113.5:8080/v1/chat/completions"
    ));
    assert!(looks_like_local_ai_endpoint(
        "https://my-ollama.example/v1/completions"
    ));
}

#[test]
fn local_ai_rejects_bare_loopback_with_random_port() {
    assert!(!looks_like_local_ai_endpoint("http://127.0.0.1:54321"));
    assert!(!looks_like_local_ai_endpoint("http://127.0.0.1:42000/"));
    assert!(!looks_like_local_ai_endpoint("http://localhost:33333"));
    assert!(!looks_like_local_ai_endpoint("http://[::1]:51234"));
}

#[test]
fn local_ai_matches_private_lan_hosts() {
    assert!(looks_like_local_ai_endpoint(
        "http://192.168.1.100:11434/v1"
    ));
    assert!(looks_like_local_ai_endpoint("http://10.0.0.5:8080/v1"));
    assert!(looks_like_local_ai_endpoint("http://172.16.0.42:8000"));
}

#[test]
fn local_ai_rejects_real_backends() {
    assert!(!looks_like_local_ai_endpoint("https://api.tinyhumans.ai"));
    assert!(!looks_like_local_ai_endpoint(
        "https://staging-api.tinyhumans.ai"
    ));
    // OpenAI public API exposes /v1 as a version prefix — must NOT match.
    assert!(!looks_like_local_ai_endpoint("https://api.openai.com/v1"));
    assert!(!looks_like_local_ai_endpoint(
        "https://my-backend.example/v1"
    ));
}

#[test]
fn local_ai_rejects_substring_path_false_positives() {
    // Earlier version used `contains` — these are the regressions it caused.
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
fn local_ai_handles_garbage_input() {
    assert!(!looks_like_local_ai_endpoint(""));
    assert!(!looks_like_local_ai_endpoint("   "));
    assert!(!looks_like_local_ai_endpoint("not a url"));
    assert!(!looks_like_local_ai_endpoint("/v1/chat/completions")); // relative — must not panic
}

#[test]
fn local_ai_matches_lm_studio_default_port() {
    assert!(looks_like_local_ai_endpoint("http://localhost:1234"));
    assert!(looks_like_local_ai_endpoint("http://127.0.0.1:1234"));
    assert!(looks_like_local_ai_endpoint(
        "http://127.0.0.1:1234/v1/chat/completions"
    ));
}

#[test]
fn local_ai_matches_v1_subpath_on_loopback() {
    assert!(looks_like_local_ai_endpoint(
        "http://localhost:11434/v1/models"
    ));
    assert!(looks_like_local_ai_endpoint(
        "http://127.0.0.1:8080/v1/embeddings"
    ));
}

// ── openhuman_backend detection ───────────────────────────────────────────

#[test]
fn openhuman_backend_detection_accepts_hosted_api_paths() {
    assert!(looks_like_openhuman_backend_endpoint(
        "https://api.tinyhumans.ai/openai/v1/chat/completions"
    ));
    assert!(looks_like_openhuman_backend_endpoint(
        "https://staging-api.tinyhumans.ai/openai/v1/chat/completions"
    ));
    assert!(!looks_like_openhuman_backend_endpoint(
        "https://openrouter.ai/api/v1/chat/completions"
    ));
    assert!(!looks_like_openhuman_backend_endpoint(
        "http://localhost:1234/v1/chat/completions"
    ));
}

// ── effective_backend_api_url ─────────────────────────────────────────────

#[test]
fn backend_url_handles_llm_endpoint_overrides() {
    let _guard = env_lock();
    let _env = EnvSnapshot::clear_backend_env();
    let fallback = fallback_backend_base_for_current_build();

    let cases: &[(&str, &str)] = &[
        (
            "https://api.tinyhumans.ai/openai/v1/chat/completions",
            "https://api.tinyhumans.ai",
        ),
        ("http://localhost:11434/v1/chat/completions", &fallback),
        ("https://api.tinyhumans.ai", "https://api.tinyhumans.ai"),
        (
            "https://api.tinyhumans.ai/openai/v1/",
            "https://api.tinyhumans.ai",
        ),
        ("https://openrouter.ai/api/v1/chat/completions", &fallback),
    ];

    for (api_url, expected) in cases {
        assert_eq!(
            effective_backend_api_url(&Some(api_url.to_string())),
            *expected,
            "api_url = {api_url}"
        );
    }
}

#[test]
fn backend_url_falls_back_for_local_ai_override() {
    let _guard = env_lock();
    let _env = EnvSnapshot::clear_backend_env();
    let expected = fallback_backend_base_for_current_build();

    assert_eq!(
        effective_backend_api_url(&Some("http://127.0.0.1:11434/v1".to_string())),
        expected
    );
}

#[test]
fn backend_url_falls_back_for_cloud_inference_base() {
    // Regression: TAURI-RUST-HW1. A BYO user whose `api_url` is a public
    // cloud-inference provider's *canonical base* (no `/chat/completions`
    // path, public host) must NOT have backend domain calls routed there —
    // `GET /teams/me/usage` was 400ing against `openrouter.ai`.
    let _guard = env_lock();
    let _env = EnvSnapshot::clear_backend_env();
    let fallback = fallback_backend_base_for_current_build();

    let falls_back: &[&str] = &[
        "https://openrouter.ai/api/v1",
        "https://api.atlascloud.ai/v1",
        "https://api.openai.com/v1",
        "https://api.groq.com/openai/v1",
        "https://generativelanguage.googleapis.com/v1beta/openai",
    ];
    for api_url in falls_back {
        assert_eq!(
            effective_backend_api_url(&Some(api_url.to_string())),
            fallback,
            "cloud inference base must fall back: {api_url}"
        );
    }

    // Our own hosted backend still passes through (is_openhuman short-circuit),
    // but an UNKNOWN custom backend at a bare `/v1` base is now classified as
    // an OpenAI-compatible inference base (#4153, Signal 2) and falls back so
    // control-plane calls are not misrouted. A self-hosted backend must use a
    // non-`/v1` base (see the `my-openhuman.example.com` case) to keep routing.
    assert_eq!(
        effective_backend_api_url(&Some("https://api.tinyhumans.ai/v1".to_string())),
        "https://api.tinyhumans.ai",
        "openhuman backend host must pass through"
    );
    assert_eq!(
        effective_backend_api_url(&Some("https://my-backend.example/v1".to_string())),
        fallback,
        "unknown bare-/v1 base is an inference base and must fall back"
    );
}

#[test]
fn backend_url_recognizes_hosted_openhuman_inference_host() {
    let _guard = env_lock();
    let _env = EnvSnapshot::clear_backend_env();
    let fallback = fallback_backend_base_for_current_build();

    for api_url in [
        "https://api.openhuman.ai/v1",
        "https://api.openhuman.ai/custom/inference",
    ] {
        assert_eq!(
            effective_backend_api_url(&Some(api_url.to_string())),
            fallback,
            "hosted inference host must not become the backend API base: {api_url}"
        );
    }
}

#[test]
fn backend_url_falls_back_to_env_when_override_is_local_ai() {
    let _guard = env_lock();
    let _env = EnvSnapshot::clear_backend_env();
    std::env::set_var("BACKEND_URL", "https://staging-api.tinyhumans.ai/");

    assert_eq!(
        effective_backend_api_url(&Some(
            "http://127.0.0.1:8080/v1/chat/completions".to_string()
        )),
        "https://staging-api.tinyhumans.ai"
    );
}

#[test]
fn backend_url_keeps_real_backend_override() {
    assert_eq!(
        effective_backend_api_url(&Some("https://staging-api.tinyhumans.ai/".to_string())),
        "https://staging-api.tinyhumans.ai"
    );
}

#[test]
fn backend_url_without_override_matches_effective_api_url() {
    let _guard = env_lock();
    let _env = EnvSnapshot::clear_backend_env();
    assert_eq!(effective_backend_api_url(&None), effective_api_url(&None));
}

// ── GH #4153: remote managed inference providers parked in `api_url` ──────

#[test]
fn inference_provider_matches_known_remote_hosts() {
    // The hosts the #4058 `host` tag pinned the `/teams/me/usage` flood to.
    assert!(looks_like_inference_provider_endpoint(
        "https://openrouter.ai/api/v1"
    ));
    assert!(looks_like_inference_provider_endpoint(
        "https://api.openmodel.ai/v1"
    ));
    assert!(looks_like_inference_provider_endpoint(
        "https://api.atlascloud.ai/v1"
    ));
    // Other managed providers, apex and subdomain.
    assert!(looks_like_inference_provider_endpoint(
        "https://api.openai.com/v1"
    ));
    assert!(looks_like_inference_provider_endpoint(
        "https://api.groq.com/openai/v1"
    ));
    assert!(looks_like_inference_provider_endpoint(
        "https://api.mistral.ai"
    ));
}

#[test]
fn inference_provider_matches_bare_v1_base_on_unknown_host() {
    // An unknown OpenAI-compatible provider, recognised by its `/v1` base.
    assert!(looks_like_inference_provider_endpoint(
        "https://llm.unknown-provider.example/v1"
    ));
    assert!(looks_like_inference_provider_endpoint(
        "https://gw.example.test/api/v1/"
    ));
}

#[test]
fn inference_provider_excludes_openhuman_backend_and_plain_hosts() {
    // Our own hosted backend is a backend, even though it serves inference.
    assert!(!looks_like_inference_provider_endpoint(
        "https://api.tinyhumans.ai/openai/v1/chat/completions"
    ));
    assert!(!looks_like_inference_provider_endpoint(
        "https://staging-api.tinyhumans.ai/"
    ));
    // A custom self-hosted OpenHuman backend (no provider host, no `/v1`
    // base) must keep routing control-plane calls to itself.
    assert!(!looks_like_inference_provider_endpoint(
        "https://my-openhuman.example.com/"
    ));
    // Garbage / relative input never panics or matches.
    assert!(!looks_like_inference_provider_endpoint(""));
    assert!(!looks_like_inference_provider_endpoint("not a url"));
}

#[test]
fn backend_url_falls_back_for_remote_inference_provider_override() {
    // The core of #4153: `config.api_url` set to a managed inference
    // provider must NOT be used as the control-plane base; backend calls
    // fall back to the canonical default chain.
    let _guard = env_lock();
    let _env = EnvSnapshot::clear_backend_env();
    let expected = fallback_backend_base_for_current_build();

    assert_eq!(
        effective_backend_api_url(&Some("https://openrouter.ai/api/v1".to_string())),
        expected
    );
    assert_eq!(
        effective_backend_api_url(&Some("https://api.openmodel.ai/v1".to_string())),
        expected
    );
    assert_eq!(
        effective_backend_api_url(&Some("https://api.atlascloud.ai/v1".to_string())),
        expected
    );
}

#[test]
fn backend_url_falls_back_to_env_for_remote_inference_provider() {
    let _guard = env_lock();
    let _env = EnvSnapshot::clear_backend_env();
    std::env::set_var("BACKEND_URL", "https://staging-api.tinyhumans.ai/");

    assert_eq!(
        effective_backend_api_url(&Some("https://openrouter.ai/api/v1".to_string())),
        "https://staging-api.tinyhumans.ai"
    );
}

#[test]
fn backend_url_strips_inference_path_from_env() {
    // Regression: OPENHUMAN-TAURI-H6 / -HN, issue #2075.
    let _guard = env_lock();
    let _env = EnvSnapshot::clear_backend_env();
    std::env::set_var(
        "BACKEND_URL",
        "https://api.tinyhumans.ai/openai/v1/chat/completions",
    );

    assert_eq!(
        effective_backend_api_url(&None),
        "https://api.tinyhumans.ai"
    );
}
