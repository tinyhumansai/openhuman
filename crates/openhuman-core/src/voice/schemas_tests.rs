use super::*;
use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
    expect_schema_required_inputs,
};
use serde_json::json;

#[test]
fn legacy_voice_catalog_keeps_its_handlers_and_schema_fixtures() {
    use super::handlers::*;
    use crate::core::all::ControllerHandler;

    let controllers: Vec<_> = all_voice_registered_controllers()
        .into_iter()
        .filter(|controller| !controller.schema.method_name().contains(".live_"))
        .collect();
    let expected: [(&str, ControllerHandler); 11] = [
        ("voice.status", handle_voice_status),
        ("voice.transcribe", handle_voice_transcribe),
        ("voice.transcribe_bytes", handle_voice_transcribe_bytes),
        ("voice.tts", handle_voice_tts),
        ("voice.reply_synthesize", handle_voice_reply_synthesize),
        ("voice.stt_dispatch", handle_voice_stt_dispatch),
        ("voice.tts_dispatch", handle_voice_tts_dispatch),
        ("voice.set_providers", handle_voice_set_providers),
        (
            "voice.update_provider_settings",
            handle_voice_update_provider_settings,
        ),
        ("voice.list_models", handle_voice_list_models),
        ("voice.test_provider", handle_voice_test_provider),
    ];
    assert_eq!(controllers.len(), expected.len());
    for (controller, (method, handler)) in controllers.iter().zip(expected) {
        assert_eq!(controller.schema.method_name(), method);
        assert!(std::ptr::fn_addr_eq(controller.handler, handler));
    }

    let contracts = crate::voice::parameter_contracts();
    for contract in contracts
        .iter()
        .filter(|contract| !contract.method.contains(".live_"))
    {
        let controller = controllers
            .iter()
            .find(|controller| controller.schema.method_name() == contract.method)
            .expect("each core voice schema fixture has a legacy row");
        crate::core::all::verify_parameter_schema(&controller.schema, contract)
            .expect("legacy core voice schema retains its independent fixture");
    }
}

#[test]
fn voice_schema_wire_contracts_are_preserved() {
    let identities = [
        ("voice_status", "status"),
        ("voice_transcribe", "transcribe"),
        ("voice_transcribe_bytes", "transcribe_bytes"),
        ("voice_tts", "tts"),
        ("voice_stt_dispatch", "stt_dispatch"),
        ("voice_tts_dispatch", "tts_dispatch"),
        ("voice_set_providers", "set_providers"),
        ("voice_update_provider_settings", "update_provider_settings"),
        ("voice_list_models", "list_models"),
        ("voice_test_provider", "test_provider"),
    ];
    for (key, function) in identities {
        expect_schema_identity(&voice_schemas(key), "voice", function);
    }
    let fields: &[(&str, &[(&str, bool)])] = &[
        ("voice_transcribe", &[("audio_path", true)]),
        (
            "voice_transcribe_bytes",
            &[("audio_bytes", true), ("extension", false)],
        ),
        ("voice_tts", &[("text", true), ("output_path", false)]),
        (
            "voice_update_provider_settings",
            &[
                ("voice_providers", false),
                ("stt_provider", false),
                ("tts_provider", false),
            ],
        ),
        ("voice_list_models", &[("provider_id", true)]),
        (
            "voice_test_provider",
            &[("workload", true), ("provider", true)],
        ),
    ];
    for (key, expected) in fields {
        let expected: Vec<_> = expected
            .iter()
            .map(|(name, required)| (*name, Some(*required)))
            .collect();
        expect_schema_fields(&voice_schemas(key), &expected);
    }
    assert!(voice_schemas("voice_status").inputs.is_empty());
    expect_schema_required_inputs(&voice_schemas("voice_update_provider_settings"), &[]);
    assert_schema_controller_parity(
        &all_voice_controller_schemas(),
        &all_voice_registered_controllers(),
        "status",
    );
}

#[test]
fn deserialize_params_applies_defaults() {
    let params = Map::from_iter([
        ("audio_path".to_string(), json!("/tmp/audio.wav")),
        ("context".to_string(), Value::Null),
    ]);

    let parsed = deserialize_params::<TranscribeParams>(params).expect("parse transcribe");
    assert_eq!(parsed.audio_path, "/tmp/audio.wav");
    assert_eq!(parsed.context, None);
    assert!(!parsed.skip_cleanup);
}

#[test]
fn deserialize_params_rejects_wrong_type() {
    let params = Map::from_iter([("audio_bytes".to_string(), json!("not-bytes"))]);
    let err =
        deserialize_params::<TranscribeBytesParams>(params).expect_err("wrong type should fail");
    assert!(err.contains("invalid params"));
}

#[tokio::test]
async fn voice_server_start_reports_method_not_found() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    let workspace = tempfile::tempdir().expect("private workspace");
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.action_dir = workspace.path().to_path_buf();
    config.modules.enabled = false;
    config.modules.allow_download = false;
    let core = CoreContext::for_test_with_config(DomainSet::full(), config);
    let result = crate::core::invoke::invoke_method(
        DispatchCtx::new(core),
        crate::core::types::AppState {
            core_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        "openhuman.voice_server_start",
        Value::Object(Map::new()),
    )
    .await;
    assert!(
        result
            .expect_err("retired method must be reported as unknown")
            .starts_with(crate::core::dispatch::UNKNOWN_METHOD_PREFIX),
        "retired RPC must be indistinguishable from an unknown method"
    );
}

#[test]
fn deserialize_voice_transcribe_with_all_fields() {
    let params = Map::from_iter([
        ("audio_path".to_string(), json!("/tmp/a.wav")),
        ("context".to_string(), json!("hello")),
        ("skip_cleanup".to_string(), json!(true)),
    ]);
    let parsed: TranscribeParams = deserialize_params(params).unwrap();
    assert_eq!(parsed.audio_path, "/tmp/a.wav");
    assert_eq!(parsed.context.as_deref(), Some("hello"));
    assert!(parsed.skip_cleanup);
}

#[test]
fn deserialize_voice_tts_requires_text() {
    let params = Map::new();
    let err = deserialize_params::<TtsParams>(params).unwrap_err();
    assert!(err.contains("invalid params"));
}

#[test]
fn deserialize_voice_tts_accepts_optional_output_path() {
    let params = Map::from_iter([
        ("text".to_string(), json!("hello world")),
        ("output_path".to_string(), json!("/tmp/out.wav")),
    ]);
    let parsed: TtsParams = deserialize_params(params).unwrap();
    assert_eq!(parsed.text, "hello world");
    assert_eq!(parsed.output_path.as_deref(), Some("/tmp/out.wav"));
}

// ---------------------------------------------------------------------------
// New voice provider registry RPC tests
// ---------------------------------------------------------------------------
#[test]
fn new_rpcs_are_in_registry() {
    let registry = all_voice_registered_controllers();
    let functions: Vec<&'static str> = registry.iter().map(|c| c.schema.function).collect();
    assert!(
        functions.contains(&"update_provider_settings"),
        "voice.update_provider_settings must be registered"
    );
    assert!(
        functions.contains(&"list_models"),
        "voice.list_models must be registered"
    );
    assert!(
        functions.contains(&"test_provider"),
        "voice.test_provider must be registered"
    );
}

#[test]
fn validate_stt_provider_accepts_sentinels() {
    assert!(validate_stt_provider("cloud").is_ok());
    assert!(validate_stt_provider("openhuman").is_ok());
    assert!(validate_stt_provider("backend").is_ok());
}

#[test]
fn validate_stt_provider_rejects_removed_local_engine() {
    for removed in ["whisper", "local"] {
        let err = validate_stt_provider(removed)
            .expect_err("the bundled whisper.cpp engine is gone — must not be settable");
        assert!(
            err.contains("removed"),
            "error should say the engine was removed, got: {err}"
        );
    }
}

#[test]
fn validate_stt_provider_accepts_slug_grammar() {
    assert!(validate_stt_provider("deepgram:nova-2").is_ok());
    assert!(validate_stt_provider("openai:whisper-1").is_ok());
    assert!(validate_stt_provider("custom").is_ok()); // bare slug
}

#[test]
fn validate_tts_provider_accepts_sentinels() {
    assert!(validate_tts_provider("cloud").is_ok());
    assert!(validate_tts_provider("openhuman").is_ok());
    assert!(validate_tts_provider("piper").is_ok());
}

#[test]
fn validate_tts_provider_accepts_slug_grammar() {
    assert!(validate_tts_provider("openai:alloy").is_ok());
    assert!(validate_tts_provider("elevenlabs:voice-id").is_ok());
    assert!(validate_tts_provider("custom").is_ok());
}

#[test]
fn deserialize_voice_update_provider_settings_params() {
    let params = Map::from_iter([
        (
            "voice_providers".to_string(),
            json!([{
                "slug": "deepgram",
                "endpoint": "https://api.deepgram.com/v1",
                "capability": "stt",
                "stt_api_style": "deepgram"
            }]),
        ),
        ("stt_provider".to_string(), json!("deepgram:nova-2")),
    ]);
    let parsed = deserialize_params::<VoiceUpdateProviderSettingsParams>(params).unwrap();
    assert_eq!(parsed.stt_provider, Some("deepgram:nova-2".into()));
    assert!(parsed.voice_providers.is_some());
    let providers = parsed.voice_providers.unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].slug, "deepgram");
}

#[test]
fn deserialize_voice_list_models_params() {
    let params = Map::from_iter([
        ("provider_id".to_string(), json!("deepgram")),
        ("capability".to_string(), json!("stt")),
    ]);
    let parsed = deserialize_params::<VoiceListModelsParams>(params).unwrap();
    assert_eq!(parsed.provider_id, "deepgram");
    assert_eq!(parsed.capability, Some("stt".into()));
}

#[test]
fn deserialize_voice_test_provider_params() {
    let params = Map::from_iter([
        ("workload".to_string(), json!("stt")),
        ("provider".to_string(), json!("deepgram:nova-2")),
    ]);
    let parsed = deserialize_params::<VoiceTestProviderParams>(params).unwrap();
    assert_eq!(parsed.workload, "stt");
    assert_eq!(parsed.provider, "deepgram:nova-2");
    // Absent `api_key` means "use the stored credential", not "empty key".
    assert_eq!(parsed.api_key, None);
    assert!(!parsed.validate_only);
}

#[test]
fn deserialize_voice_test_provider_params_accepts_candidate_key() {
    let params = Map::from_iter([
        ("workload".to_string(), json!("stt")),
        ("provider".to_string(), json!("elevenlabs")),
        ("validate_only".to_string(), json!(true)),
        ("api_key".to_string(), json!("sk-candidate-not-yet-saved")),
    ]);
    let parsed = deserialize_params::<VoiceTestProviderParams>(params).unwrap();
    assert!(parsed.validate_only);
    assert_eq!(
        parsed.api_key.as_deref(),
        Some("sk-candidate-not-yet-saved")
    );
}

#[test]
fn voice_test_provider_params_debug_redacts_the_candidate_key() {
    let params = Map::from_iter([
        ("workload".to_string(), json!("stt")),
        ("provider".to_string(), json!("elevenlabs")),
        ("api_key".to_string(), json!("sk-super-secret-value")),
    ]);
    let parsed = deserialize_params::<VoiceTestProviderParams>(params).unwrap();
    let rendered = format!("{parsed:?}");
    assert!(
        !rendered.contains("sk-super-secret-value"),
        "Debug must never render the raw key: {rendered}"
    );
    assert!(rendered.contains("[REDACTED]"), "got: {rendered}");
    // The non-secret fields still have to be debuggable, or the manual impl
    // has traded a leak for an undiagnosable handler.
    assert!(rendered.contains("elevenlabs"), "got: {rendered}");
}

#[test]
fn test_provider_schema_exposes_the_dry_run_candidate_key() {
    let s = voice_schemas("voice_test_provider");
    let api_key = s
        .inputs
        .iter()
        .find(|i| i.name == "api_key")
        .expect("voice_test_provider must accept a candidate api_key for the dry run");
    assert!(
        !api_key.required,
        "api_key is optional — omitting it means 'use the stored credential'"
    );
}

#[test]
fn generate_silent_wav_produces_valid_wav_header() {
    let wav = generate_silent_wav();
    assert!(wav.len() >= 44, "WAV must have at least 44 bytes (header)");
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(&wav[12..16], b"fmt ");
    assert_eq!(&wav[36..40], b"data");
    // 16kHz — the rate every routed STT engine takes without resampling.
    let sample_rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
    assert_eq!(sample_rate, 16_000, "fixture must be 16kHz");
    // total size = header(44) + 1600 samples * 2 bytes = 3244
    assert_eq!(wav.len(), 3244);
}

#[test]
fn stt_dispatch_params_all_optional_except_audio() {
    let params = Map::from_iter([("audio_base64".to_string(), json!("AAAA"))]);
    let parsed = deserialize_params::<SttDispatchParams>(params).unwrap();
    assert_eq!(parsed.audio_base64, "AAAA");
    assert!(parsed.provider.is_none());
    assert!(parsed.model.is_none());
    assert!(parsed.mime_type.is_none());
    assert!(parsed.file_name.is_none());
    assert!(parsed.language.is_none());
}

#[test]
fn tts_dispatch_params_all_optional_except_text() {
    let params = Map::from_iter([("text".to_string(), json!("hello"))]);
    let parsed = deserialize_params::<TtsDispatchParams>(params).unwrap();
    assert_eq!(parsed.text, "hello");
    assert!(parsed.provider.is_none());
    assert!(parsed.voice.is_none());
}

#[test]
fn set_providers_params_all_optional() {
    let parsed = deserialize_params::<SetProvidersParams>(Map::new()).unwrap();
    assert!(parsed.stt_provider.is_none());
    assert!(parsed.tts_provider.is_none());
    assert!(parsed.stt_model.is_none());
    assert!(parsed.tts_voice.is_none());
}

// ── `validate_provider_key`: the candidate-key half of #5947 ────────────────
//
// `validate_provider_key` had no test caller at all — the only references in
// the tree were its definition and the one handler call site. What that left
// unpinned is the branch the "Test Key" button exists for: a key the user has
// just typed, for a provider they have not saved yet, must be the key that
// gets validated. Invert the two match arms and the modal silently validates
// whatever was already stored (or reports "no API key configured" for a key the
// user is looking at), while still answering `ok` — a wrong answer about a
// credential, which is the worst shape this code can fail in.
//
// `127.0.0.1:1` is the observation trick: it refuses immediately, so "the
// validator got past the key check and tried to reach the provider" is
// distinguishable from "the validator stopped at the key check" with no mock
// server, no network and no wall-clock cost.

fn candidate_precedence_config(slug: &str) -> crate::config::Config {
    use crate::config::schema::voice_providers::VoiceProviderCreds;

    let mut config = crate::config::Config::default();
    config.voice_providers.push(VoiceProviderCreds {
        slug: slug.to_string(),
        endpoint: "http://127.0.0.1:1".to_string(),
        ..Default::default()
    });
    config
}

/// A candidate key must win over an absent stored key (#5947, #5896).
#[tokio::test]
async fn validate_provider_key_prefers_the_candidate_over_an_absent_stored_key() {
    let slug = "e2e-candidate-precedence";
    let config = candidate_precedence_config(slug);

    // Nothing stored, no candidate: the validator must stop at the key check.
    // This is the control — it proves the slug resolves and that the "no key"
    // answer is what the absent-key path produces.
    let without = super::helpers::validate_provider_key(slug, &config, None)
        .await
        .expect_err("no key anywhere cannot be a valid provider");
    assert!(
        without.contains("no API key configured"),
        "control: with nothing stored and no candidate the validator must stop \
         at the key check, got {without:?}"
    );

    // Same config, same absent stored key — only a candidate is added.
    let with =
        super::helpers::validate_provider_key(slug, &config, Some("candidate-not-yet-saved"))
            .await
            .expect_err("127.0.0.1:1 refuses, so the request cannot succeed");
    assert!(
        !with.contains("no API key configured"),
        "a candidate key must be used when nothing is stored for the slug; \
         got {with:?}, which means the candidate was ignored and the absent \
         stored key won — the modal would report on a key the user never typed"
    );
    assert!(
        with.contains("request failed"),
        "the candidate must carry the validator through to the provider request, got {with:?}"
    );
}
