use super::*;
use crate::config::schema::voice_live::LIVE_PROVIDER_SARVAM;

#[test]
fn providers_and_settings_reflect_the_config() {
    let config = Config::default();
    let providers = live_providers(&config).value;
    assert_eq!(
        providers.default_provider,
        config.voice_live.default_provider
    );
    // Only the bring-your-own-key providers are built in; hosted ones register.
    assert_eq!(providers.providers.len(), 2);
    assert_eq!(live_settings_get(&config).value, config.voice_live);
}

#[test]
fn patches_apply_only_the_given_fields_and_validate_ids() {
    let mut config = Config::default();
    let before_gemini = config.voice_live.gemini.clone();
    apply_patch(
        &mut config,
        LiveSettingsPatch {
            default_provider: Some(LIVE_PROVIDER_SARVAM.into()),
            sarvam: Some(SarvamLiveSettingsPatch {
                language: Some(Some("hi-IN".into())),
                ..SarvamLiveSettingsPatch::default()
            }),
            ..LiveSettingsPatch::default()
        },
    )
    .unwrap();
    assert_eq!(config.voice_live.default_provider, LIVE_PROVIDER_SARVAM);
    assert_eq!(config.voice_live.sarvam.language.as_deref(), Some("hi-IN"));
    assert_eq!(config.voice_live.gemini, before_gemini);
    apply_patch(
        &mut config,
        LiveSettingsPatch {
            gemini: Some(GeminiLiveSettingsPatch::default()),
            elevenlabs: Some(Default::default()),
            ..LiveSettingsPatch::default()
        },
    )
    .unwrap();
    let err = apply_patch(
        &mut config,
        LiveSettingsPatch {
            default_provider: Some("nope".into()),
            ..LiveSettingsPatch::default()
        },
    )
    .unwrap_err();
    assert!(err.contains("nope"));
}

#[test]
fn provider_patches_preserve_omitted_fields_and_clear_explicit_nulls() {
    let mut config = Config::default();
    apply_patch(
        &mut config,
        LiveSettingsPatch {
            gemini: Some(GeminiLiveSettingsPatch {
                voice: Some(Some("Puck".into())),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .unwrap();
    apply_patch(
        &mut config,
        LiveSettingsPatch {
            gemini: Some(GeminiLiveSettingsPatch {
                language: Some(Some("en-US".into())),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(config.voice_live.gemini.voice.as_deref(), Some("Puck"));
    assert_eq!(config.voice_live.gemini.language.as_deref(), Some("en-US"));

    apply_patch(
        &mut config,
        LiveSettingsPatch {
            gemini: Some(GeminiLiveSettingsPatch {
                voice: Some(None),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(config.voice_live.gemini.voice, None);
    assert_eq!(config.voice_live.gemini.language.as_deref(), Some("en-US"));
}

#[tokio::test]
async fn settings_set_saves_to_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    config.config_path = dir.path().join("config.toml");
    let saved = live_settings_set(
        &mut config,
        LiveSettingsPatch {
            default_provider: Some(LIVE_PROVIDER_SARVAM.into()),
            ..LiveSettingsPatch::default()
        },
    )
    .await
    .unwrap()
    .value;
    assert_eq!(saved.default_provider, LIVE_PROVIDER_SARVAM);
    let on_disk = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(on_disk.contains("[voice_live]"));
}

#[tokio::test]
async fn testing_an_unconfigured_provider_reports_why() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    config.config_path = dir.path().join("config.toml");
    let result = live_test_provider(&config, LIVE_PROVIDER_SARVAM)
        .await
        .value;
    assert!(!result.ok);
    assert!(result.error.unwrap().contains("not_configured"));
    let result = live_test_provider(&config, "nope").await.value;
    assert!(!result.ok);
}
