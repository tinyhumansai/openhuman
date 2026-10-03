//! Tests for the surrounding module.

use super::*;

#[test]
fn defaults_keep_local_runtime_and_every_usage_gate_off() {
    let cfg = LocalAiConfig::default();
    assert!(!cfg.is_active());
    #[allow(deprecated)]
    {
        assert!(!cfg.use_local_for_embeddings());
        assert!(!cfg.use_local_for_heartbeat());
        assert!(!cfg.use_local_for_subconscious());
    }
    assert_eq!(cfg.embedding_model_id, "bge-m3");
}

#[test]
fn debug_output_redacts_the_api_key() {
    let cfg = LocalAiConfig {
        api_key: Some("secret-key".into()),
        ..Default::default()
    };
    let debug = format!("{cfg:?}");
    assert!(!debug.contains("secret-key"));
    assert!(debug.contains("<redacted>"));
    assert!(debug.contains("voice_llm_cleanup_enabled: true"));
    assert!(debug.contains("usage: LocalAiUsage"));
}

#[test]
fn legacy_enabled_key_does_not_reenable_the_runtime() {
    let cfg: LocalAiConfig = toml::from_str("enabled = true").unwrap();
    assert!(!cfg.runtime_enabled);
}
