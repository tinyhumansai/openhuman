use super::*;

#[tokio::test]
async fn choosing_backend_resets_a_legacy_nondefault_stt_engine() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.config_path = dir.path().join("config.toml");
    config.workspace_dir = dir.path().join("workspace");
    config.voice_server.stt_engine = crate::config::SttEngine::Elevenlabs;
    config.save().await.unwrap();
    let ctx = DispatchCtx::new(crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config.clone(),
    ));

    let custom = Map::from_iter([(
        "stt_provider".to_string(),
        Value::String("custom-provider:model-v2".into()),
    )]);
    handle_voice_set_providers(ctx.clone(), custom)
        .await
        .expect("setting a custom provider should succeed");
    let persisted =
        crate::config::Config::load_from_config_path(&config.config_path, &config.workspace_dir)
            .await
            .unwrap();
    assert_eq!(
        persisted.voice_server.stt_engine,
        crate::config::SttEngine::Elevenlabs
    );
    assert_eq!(
        crate::voice::effective_stt_provider(&persisted),
        "custom-provider:model-v2"
    );

    for alias in ["cloud", "backend", "openhuman"] {
        let params = Map::from_iter([("stt_provider".to_string(), Value::String(alias.into()))]);
        handle_voice_set_providers(ctx.clone(), params)
            .await
            .expect("setting a backend alias should succeed");

        let persisted = crate::config::Config::load_from_config_path(
            &config.config_path,
            &config.workspace_dir,
        )
        .await
        .unwrap();
        assert_eq!(
            persisted.voice_server.stt_engine,
            crate::config::SttEngine::Backend
        );
        assert_eq!(crate::voice::effective_stt_provider(&persisted), "cloud");
    }
}

#[test]
fn stt_test_module_wait_stays_under_the_frontend_abort() {
    // `testVoiceProvider` aborts at 30 s; the bounded module wait must answer
    // first so the user sees the still-loading result, not a client timeout.
    assert!(STT_TEST_MODULE_LOAD_WAIT > std::time::Duration::ZERO);
    assert!(STT_TEST_MODULE_LOAD_WAIT <= std::time::Duration::from_secs(25));
}
