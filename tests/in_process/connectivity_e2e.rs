//! In-process JSON-RPC E2E coverage for connectivity.

use crate::config_auth_harness::*;

#[test]
fn api_config_url_resolution_classifies_backend_and_inference_paths() {
    let _lock = env_lock();
    // Integration-test binaries link the library compiled WITHOUT `cfg(test)`,
    // so `compile_time_api_base_env_values()` resolves `option_env!("BACKEND_URL")`
    // / `option_env!("VITE_BACKEND_URL")`. The mock test harness bakes
    // `BACKEND_URL` at build time, which would make the blank / local-AI override
    // fall-throughs below resolve to the baked URL instead of the compile
    // default. Pin `BACKEND_URL` at runtime — runtime resolution wins over the
    // compile-time bake — so env/default resolution is deterministic regardless
    // of what CI baked into the binary.
    let _backend = EnvVarGuard::set("BACKEND_URL", DEFAULT_API_BASE_URL);
    let _vite_backend = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _app_env = EnvVarGuard::unset(APP_ENV_VAR);
    let _vite_app_env = EnvVarGuard::unset(VITE_APP_ENV_VAR);

    assert_eq!(
        normalize_api_base_url(" https://api.example.test/// "),
        "https://api.example.test"
    );
    assert_eq!(
        api_url(
            "https://api.tinyhumans.ai/openai/v1/chat/completions",
            "/auth/me"
        ),
        "https://api.tinyhumans.ai/auth/me"
    );
    assert_eq!(api_url("not a url/", "auth/me"), "not a url/auth/me");
    assert_eq!(api_url("not a url/", "/auth/me"), "not a url/auth/me");
    assert_eq!(
        api_url(" https://api.tinyhumans.ai/ ", ""),
        "https://api.tinyhumans.ai"
    );

    assert!(!looks_like_local_ai_endpoint(""));
    assert!(looks_like_local_ai_endpoint("http://localhost:11434"));
    assert!(looks_like_local_ai_endpoint(
        "http://10.0.0.2/v1/chat/completions"
    ));
    assert!(looks_like_local_ai_endpoint(
        "https://api.openai.com/v1/completions"
    ));
    assert!(looks_like_local_ai_endpoint("http://0.0.0.0:8000"));
    assert!(looks_like_local_ai_endpoint("http://service.localhost/v1"));
    assert!(looks_like_local_ai_endpoint("http://192.168.1.7:9000/v1"));
    assert!(!looks_like_local_ai_endpoint("http://127.0.0.1:45678"));
    assert!(!looks_like_local_ai_endpoint("https://api.example.test/v1"));
    assert!(!looks_like_local_ai_endpoint(
        "https://api.example.test/audit/v1/chat/completions-logs"
    ));
    assert!(!looks_like_local_ai_endpoint("not a url"));

    assert_eq!(effective_api_url(&Some("   ".into())), DEFAULT_API_BASE_URL);
    assert_eq!(
        effective_api_url(&Some(" http://127.0.0.1:11434/ ".into())),
        "http://127.0.0.1:11434"
    );
    assert_eq!(
        effective_inference_url(&Some("https://api.tinyhumans.ai".into()), &None),
        format!("https://api.tinyhumans.ai{OPENHUMAN_INFERENCE_PATH}")
    );
    assert_eq!(
        effective_inference_url(
            &Some("https://api.tinyhumans.ai".into()),
            &Some(" http://127.0.0.1:11434/v1/chat/completions ".into())
        ),
        "http://127.0.0.1:11434/v1/chat/completions"
    );

    assert_eq!(
        effective_backend_api_url(&Some(" http://127.0.0.1:11434/v1 ".into())),
        DEFAULT_API_BASE_URL
    );
    assert_eq!(
        effective_backend_api_url(&Some(
            " https://api.tinyhumans.ai/openai/v1/chat/completions?x=1#frag ".into()
        )),
        "https://api.tinyhumans.ai"
    );
    assert_eq!(
        effective_backend_api_url(&Some("api.tinyhumans.ai/openai/v1/chat/completions".into())),
        "https://api.tinyhumans.ai"
    );
    assert_eq!(
        effective_backend_api_url(&Some(" http://backend.example.test/path?q=1#frag ".into())),
        "http://backend.example.test"
    );

    std::env::set_var("BACKEND_URL", "");
    std::env::set_var(
        "VITE_BACKEND_URL",
        " https://backend.example.test/openai/v1/chat/completions ",
    );
    assert_eq!(
        api_base_from_env().as_deref(),
        Some("https://backend.example.test/openai/v1/chat/completions")
    );
    assert_eq!(
        effective_backend_api_url(&None),
        "https://backend.example.test"
    );

    std::env::set_var(APP_ENV_VAR, " Staging ");
    assert_eq!(app_env_from_env().as_deref(), Some("staging"));
    assert_eq!(
        default_api_base_url_for_env(app_env_from_env().as_deref()),
        DEFAULT_STAGING_API_BASE_URL
    );

    std::env::remove_var(APP_ENV_VAR);
    std::env::set_var(VITE_APP_ENV_VAR, " Production ");
    assert_eq!(app_env_from_env().as_deref(), Some("production"));
}
