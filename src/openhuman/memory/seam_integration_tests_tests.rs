use tinymemory_api::host::{MemoryConfig, DEFAULT_CLOUD_LLM_MODEL};

use crate::openhuman::config::Config;

#[test]
fn build_provider_returns_inference_wrapper_when_default() {
    // These assert the *real* seam implementations, so they need them
    // installed — that is the whole point of living on this side.
    crate::openhuman::memory::host_impls::install_for_tests();
    let cfg = Config::default();
    let provider = tinymemory_core::chat::build_chat_provider(&cfg).unwrap();
    assert!(provider.name().contains("inference:"));
}

#[test]
fn build_chat_runtime_defaults_to_openhuman_resolved_model() {
    // These assert the *real* seam implementations, so they need them
    // installed — that is the whole point of living on this side.
    crate::openhuman::memory::host_impls::install_for_tests();
    let cfg = Config::default();
    let (_provider, model) = tinymemory_core::chat::build_chat_runtime(&cfg).unwrap();
    // The managed "summarization" tier is fixed at `summarization-v1`
    // inside `make_openhuman_backend`. DEFAULT_CLOUD_LLM_MODEL is that same
    // constant — asserted here only as the expected value, not because
    // `cloud_llm_model` is consumed (it isn't; see the test below).
    assert_eq!(model, DEFAULT_CLOUD_LLM_MODEL);
}

#[test]
fn build_chat_runtime_ignores_cloud_llm_model_on_managed() {
    // These assert the *real* seam implementations, so they need them
    // installed — that is the whole point of living on this side.
    crate::openhuman::memory::host_impls::install_for_tests();
    // The managed summarization tier is locked to `summarization-v1`;
    // `memory_tree.cloud_llm_model` is inert and must not change it (neither a
    // known tier nor a custom string leaks through).
    let mut cfg = Config::default();
    cfg.memory_tree.cloud_llm_model = Some("chat-v1".into());
    let (_provider, model) = tinymemory_core::chat::build_chat_runtime(&cfg).unwrap();
    assert_eq!(model, DEFAULT_CLOUD_LLM_MODEL);

    cfg.memory_tree.cloud_llm_model = Some("custom-summary-model".into());
    let (_provider, model) = tinymemory_core::chat::build_chat_runtime(&cfg).unwrap();
    assert_eq!(model, DEFAULT_CLOUD_LLM_MODEL);
}

#[test]
fn build_provider_returns_inference_wrapper_when_local_memory_is_configured() {
    // These assert the *real* seam implementations, so they need them
    // installed — that is the whole point of living on this side.
    crate::openhuman::memory::host_impls::install_for_tests();
    // Serialize with the process-global `test_provider_override` (see the
    // inference factory tests): while an override is active, `create_chat_model`
    // returns the mock, so an unguarded read here could race it.
    let _guard = crate::openhuman::inference::inference_test_guard();
    let mut cfg = Config::default();
    cfg.memory_provider = Some("ollama:qwen2.5:0.5b".into());
    let provider = tinymemory_core::chat::build_chat_provider(&cfg).unwrap();
    assert!(provider.name().contains("qwen2.5:0.5b"));
}

#[test]
fn build_chat_runtime_preserves_local_memory_model() {
    // These assert the *real* seam implementations, so they need them
    // installed — that is the whole point of living on this side.
    crate::openhuman::memory::host_impls::install_for_tests();
    let _guard = crate::openhuman::inference::inference_test_guard();
    let mut cfg = Config::default();
    cfg.memory_provider = Some("ollama:qwen2.5:0.5b".into());
    let (_provider, model) = tinymemory_core::chat::build_chat_runtime(&cfg).unwrap();
    assert_eq!(model, "qwen2.5:0.5b");
}

/// #1574 invariant: a config-derived `active_embedding_signature` MUST be
/// byte-identical to the live provider's `.signature()` for the same
/// (provider, model, dims). Drift here silently splits one embedding space
/// into two — copied/queried vectors would never match.
#[test]
fn active_signature_matches_live_provider_signature() {
    // These assert the *real* seam implementations, so they need them
    // installed — that is the whole point of living on this side.
    crate::openhuman::memory::host_impls::install_for_tests();
    for local in [None, Some("nomic-embed-text:latest"), Some("bge-m3")] {
        let mem = MemoryConfig::default();
        let (provider, model, dims) =
            tinymemory_core::store::effective_embedding_settings(&mem, local);
        // The host ported this selection rule verbatim (#5560); the two
        // copies are independent by design, and this is the one place a
        // divergence would be caught rather than shipped. Includes the
        // blank-local edge the rule exists for.
        for probe in [local, Some("  "), Some(" bge-m3 ")] {
            assert_eq!(
                crate::openhuman::inference::embeddings::effective_embedding_settings(&mem, probe),
                tinymemory_core::store::effective_embedding_settings(&mem, probe),
                "host and engine embedding selection diverged for {probe:?}"
            );
        }
        let live = crate::openhuman::inference::embeddings::create_embedding_provider(
            &provider, &model, dims,
        )
        .expect("provider builds for test triple");
        assert_eq!(
            tinymemory_core::store::active_embedding_signature(&mem, local),
            live.signature(),
            "config-derived signature must equal live provider signature (local={local:?})"
        );
    }
}

/// #002 FR-007 / Gray review: the doctor's `summary_tree` stage must mirror
/// `summarizer_available` exactly. With local AI off and no cloud opt-in
/// (the default), the stage reports unavailable — which is correct, since
/// cloud summarization requires explicit consent. The stage must NOT fire
/// a generic "local AI required" hard-failure; it names the opt-in gap.
#[test]
fn local_ai_off_reports_no_provider_without_cloud_opt_in() {
    // These assert the *real* seam implementations, so they need them
    // installed — that is the whole point of living on this side.
    crate::openhuman::memory::host_impls::install_for_tests();
    let _g = tinymemory_core::tree::health::test_guard();
    let tmp = tempfile::TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg.embeddings_provider = Some("ollama:bge-m3".into()); // embeddings ok
    cfg.local_ai.runtime_enabled = false; // cloud opt-in not set (default false)

    let report = tinymemory_core::tree::health::run_doctor(&cfg);
    let tree = report
        .stages
        .iter()
        .find(|s| s.stage == "summary_tree")
        .unwrap();
    // summary_tree must mirror summarizer_available precisely.
    assert_eq!(
        tree.ok,
        crate::openhuman::memory::tree::tree_runtime::ops::summarizer_available(&cfg).0,
        "summary_tree health must mirror the runtime capability check"
    );
    // Without opt-in, the note names the "no summarization provider" case.
    assert!(
        tree.note.contains("no summarization provider"),
        "unexpected summary_tree note: {}",
        tree.note
    );
}

#[cfg(feature = "modules")]
#[tokio::test]
#[cfg(feature = "modules")] // `modules::connectors` — and the route it resolves — exist only with the gate on
async fn direct_mode_config_resolves_via_module_config_at_call_time() {
    // The seam this pinned moved. `ProviderContext::execute` — the engine's
    // Composio dispatch, which routed through the host's `ComposioHost` seam
    // to decide backend vs. direct — was deleted outright by tinymemory
    // v1.13.4 along with the rest of the in-process Composio pipeline (see
    // `memory::host_impls`'s module docs). The equivalent decision now lives
    // entirely on this side: `modules::connectors::module_config` builds the
    // route the `tinyconnectors` module is configured with, and it is the
    // one and only place `composio.mode` gets turned into "which tenant".
    //
    // This asserts the same property the deleted test did — a direct-mode
    // config with an inline api_key resolves to the direct route without
    // needing (or surfacing an error about) a backend session token, which
    // was the pre-#1710 bug this area guards against.
    let tmp = tempfile::tempdir().expect("tempdir");

    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.secrets.encrypt = false;
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());
    config.save().await.expect("save fake config to disk");

    let route = crate::openhuman::modules::connectors::module_config(&config)
        .expect("direct mode with an inline api_key must resolve a route");
    assert_eq!(
        route.get("route").and_then(|v| v.as_str()),
        Some("direct"),
        "a direct-mode config must resolve the direct route, not fall back to backend: {route:?}"
    );
    assert_eq!(
        route.get("api_key").and_then(|v| v.as_str()),
        Some("test-direct-key"),
        "the direct route must carry the configured api_key: {route:?}"
    );
}
