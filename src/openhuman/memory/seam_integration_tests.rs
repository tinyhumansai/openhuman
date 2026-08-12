//! Seam integration tests — core behaviour whose *answer* is host routing.
//!
//! These moved out of `tinymemory-core` with the extraction. Each one calls
//! into the extracted crate but asserts something only the host decides: which
//! provider a role resolves to, what model id that yields, whether a
//! config-derived embedding signature matches the one the real provider
//! reports, whether Composio dispatches to the backend or direct tenant.
//!
//! A stub host could only assert itself, so they belong on this side, where the
//! real `ChatHost` / `EmbeddingHost` / `ComposioHost` implementations live.
//! See [`super::host_impls`].

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tinymemory_api::host::{MemoryConfig, DEFAULT_CLOUD_LLM_MODEL};
    use tinymemory_core::sync::composio::providers::{ComposioUsageHandle, ProviderContext};

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

    #[tokio::test]
    async fn provider_context_execute_resolves_via_factory_at_call_time() {
        // These assert the *real* seam implementations, so they need them
        // installed — that is the whole point of living on this side.
        crate::openhuman::memory::host_impls::install_for_tests();
        // Build a context against a direct-mode config (no backend
        // session token, only the inline direct api_key). The factory
        // must pick the `Direct` variant on `execute` — pre-fix the
        // `client: ComposioClient` field was always backend, so this
        // path would have surfaced a backend session lookup error
        // even with `mode = "direct"`.
        let tmp = tempfile::tempdir().expect("tempdir");

        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");
        config.workspace_dir = tmp.path().join("workspace");
        config.secrets.encrypt = false;
        config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
        config.composio.api_key = Some("test-direct-key".to_string());
        config.save().await.expect("save fake config to disk");

        let ctx = ProviderContext {
            config: Arc::new(config) as Arc<tinymemory_core::Config>,
            toolkit: "gmail".to_string(),
            connection_id: None,
            usage: ComposioUsageHandle::default(),
            max_items: None,
            sync_depth_days: None,
        };
        let res = ctx.execute("GMAIL_FETCH_EMAILS", None).await;
        // The actual HTTP call will fail in the unit-test sandbox, but
        // the error must come from the direct path — never a backend
        // session lookup, which is the smoking gun for the pre-fix bug.
        if let Err(e) = res {
            let msg = e.to_string();
            assert!(
                !msg.contains("no backend session"),
                "direct-mode execute must not surface backend session artifacts: {msg}"
            );
        }
    }
}
