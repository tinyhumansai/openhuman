use super::*;
use openhuman_embed::__host::voice_registry::unregister_live_provider;

#[test]
fn parses_wrapped_and_bare_tickets() {
    let wrapped = json!({"data": {"wsUrl": "wss://x/ws?ticket=t", "sessionId": "s", "model": "m"}});
    assert_eq!(
        parse_ticket(&wrapped).unwrap(),
        GeminiTicket {
            ws_url: "wss://x/ws?ticket=t".into(),
            session_id: "s".into(),
            model: "m".into()
        }
    );
    let bare = json!({"wsUrl": "wss://y"});
    assert_eq!(parse_ticket(&bare).unwrap().session_id, "");
    assert_eq!(
        parse_ticket(&json!({"data": {}})).unwrap_err().code,
        "backend"
    );
}

#[test]
fn gemini_hosted_config_follows_the_settings() {
    let mut config = Config::default();
    config.voice_live.gemini.voice = Some("Kore".into());
    config.voice_live.gemini.model = Some("gemini-x".into());
    let mut live = LiveConfig::new();
    configure_gemini(&config, "prompt", &mut live);
    assert_eq!(live.system_instruction.as_deref(), Some("prompt"));
    assert_eq!(live.voice.as_deref(), Some("Kore"));
    assert_eq!(live.model.as_deref(), Some("gemini-x"));
    assert!(live
        .provider_options
        .get("context_window_compression")
        .is_some());
}

#[test]
fn elevenlabs_hosted_config_owns_its_prompt_and_ignores_blank_voices() {
    let mut config = Config::default();
    config.voice_live.elevenlabs.voice_id = Some("  ".into());
    let mut live = LiveConfig::new();
    configure_elevenlabs(&config, "prompt", &mut live);
    assert_eq!(
        live.system_instruction, None,
        "the hosted agent owns its prompt"
    );
    assert_eq!(live.voice, None, "blank voice ids are ignored");
    config.voice_live.elevenlabs.voice_id = Some("v1".into());
    configure_elevenlabs(&config, "prompt", &mut live);
    assert_eq!(live.voice.as_deref(), Some("v1"));
}

#[test]
fn registration_installs_both_hosted_providers() {
    let _registry = crate::registry_test_lock();

    register();
    assert!(unregister_live_provider(LIVE_PROVIDER_GEMINI_HOSTED));
    assert!(unregister_live_provider(LIVE_PROVIDER_ELEVENLABS_HOSTED));
}

#[test]
fn hosted_providers_are_listed_ahead_of_the_built_in_ones() {
    let _registry = crate::registry_test_lock();

    // The catalogue has always listed the hosted providers first:
    // gemini-hosted, elevenlabs-hosted, gemini, sarvam.
    register();
    let listed = openhuman_embed::__host::voice_registry::live_providers(&Config::default());
    unregister_live_provider(LIVE_PROVIDER_GEMINI_HOSTED);
    unregister_live_provider(LIVE_PROVIDER_ELEVENLABS_HOSTED);
    let ids: Vec<_> = listed
        .value
        .providers
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            LIVE_PROVIDER_GEMINI_HOSTED,
            LIVE_PROVIDER_ELEVENLABS_HOSTED,
            "gemini",
            "sarvam"
        ]
    );
}

#[tokio::test]
async fn hosted_providers_report_not_configured_without_a_backend_credential() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };
    assert!(!backend_ready(&config));
    let err = prepare_gemini(config, LiveConfig::new())
        .await
        .err()
        .unwrap();
    assert_eq!(err.code, "not_configured");
}
