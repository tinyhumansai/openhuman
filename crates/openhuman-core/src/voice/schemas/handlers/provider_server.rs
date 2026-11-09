//! Handlers for provider settings, model listing, and provider testing.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::core::all::{BoundRegisteredController, ControllerFuture};

use crate::voice::schemas::helpers::{
    deserialize_params, generate_silent_wav, validate_provider_key, validate_stt_provider,
    validate_tts_provider,
};
use crate::voice::schemas::params::{
    SetProvidersParams, VoiceListModelsParams, VoiceTestProviderParams,
    VoiceUpdateProviderSettingsParams,
};

// ---------------------------------------------------------------------------
// Provider configuration handlers
// ---------------------------------------------------------------------------

pub(crate) fn voice_set_providers_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(
        super::super::registry::voice_schemas("voice_set_providers"),
        deserialize_params::<SetProvidersParams>,
        run_voice_set_providers,
    )
}

pub(crate) fn handle_voice_set_providers(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    voice_set_providers_controller().invoke(ctx, params)
}

fn run_voice_set_providers(ctx: DispatchCtx, p: SetProvidersParams) -> ControllerFuture {
    Box::pin(async move {
        let mut config = ctx.load_config().await?;

        if let Some(stt) = p
            .stt_provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            validate_stt_provider(stt)?;
            config.local_ai.stt_provider = stt.to_string();
            config.stt_provider = Some(stt.to_string());
            sync_legacy_stt_engine(&mut config, stt);
        }
        if let Some(tts) = p
            .tts_provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            validate_tts_provider(tts)?;
            config.local_ai.tts_provider = tts.to_string();
            config.tts_provider = Some(tts.to_string());
        }
        if let Some(model) = p
            .stt_model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            config.local_ai.stt_model_id = model.to_string();
        }
        if let Some(voice) = p
            .tts_voice
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            config.local_ai.tts_voice_id = voice.to_string();
        }

        config
            .save()
            .await
            .map_err(crate::config::ops::config_save_error)?;
        log::debug!(
            "[voice-factory] persisted providers stt={} tts={} stt_model={} tts_voice={}",
            config.local_ai.stt_provider,
            config.local_ai.tts_provider,
            config.local_ai.stt_model_id,
            config.local_ai.tts_voice_id
        );

        Ok(serde_json::json!({
            "stt_provider": config.local_ai.stt_provider,
            "tts_provider": config.local_ai.tts_provider,
            "stt_model_id": config.local_ai.stt_model_id,
            "tts_voice_id": config.local_ai.tts_voice_id,
        }))
    })
}

pub(crate) fn voice_update_provider_settings_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(
        super::super::registry::voice_schemas("voice_update_provider_settings"),
        deserialize_params::<VoiceUpdateProviderSettingsParams>,
        run_voice_update_provider_settings,
    )
}

pub(crate) fn handle_voice_update_provider_settings(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    voice_update_provider_settings_controller().invoke(ctx, params)
}

fn run_voice_update_provider_settings(
    ctx: DispatchCtx,
    p: VoiceUpdateProviderSettingsParams,
) -> ControllerFuture {
    Box::pin(async move {
        use crate::config::schema::voice_providers::{
            generate_voice_provider_id, is_voice_slug_reserved, VoiceCapability, VoiceProviderCreds,
        };
        use tinyinference_voice::external_stt::SttApiStyle;
        use tinyinference_voice::external_tts::TtsApiStyle;

        let mut config = ctx.load_config().await?;

        if let Some(providers) = p.voice_providers {
            let mut new_entries = Vec::with_capacity(providers.len());
            for update in providers {
                let slug = update.slug.trim().to_lowercase();
                if is_voice_slug_reserved(&slug) {
                    return Err(format!(
                        "slug '{}' is reserved and cannot be used for a voice provider",
                        slug
                    ));
                }

                let capability = match update.capability.as_deref() {
                    Some("stt") => VoiceCapability::Stt,
                    Some("tts") => VoiceCapability::Tts,
                    Some("both") | None => VoiceCapability::Both,
                    Some(other) => {
                        return Err(format!(
                            "invalid capability '{other}' (valid: 'stt', 'tts', 'both')"
                        ))
                    }
                };

                let auth_style = match update.auth_style.as_deref() {
                    Some("bearer") | None => crate::config::schema::AuthStyle::Bearer,
                    Some("none") => crate::config::schema::AuthStyle::None,
                    Some(other) => {
                        return Err(format!(
                        "invalid auth_style '{other}' for voice provider (valid: 'bearer', 'none')"
                    ))
                    }
                };

                let stt_api_style = match update.stt_api_style.as_deref() {
                    Some("deepgram") => SttApiStyle::Deepgram,
                    Some("elevenlabs") => SttApiStyle::ElevenLabs,
                    Some("openai_audio") | None => SttApiStyle::OpenaiAudio,
                    Some(other) => {
                        return Err(format!(
                            "invalid stt_api_style '{other}' (valid: 'openai_audio', 'deepgram', 'elevenlabs')"
                        ))
                    }
                };

                let tts_api_style = match update.tts_api_style.as_deref() {
                    Some("elevenlabs") => TtsApiStyle::ElevenLabs,
                    Some("openai_audio") | None => TtsApiStyle::OpenaiAudio,
                    Some(other) => {
                        return Err(format!(
                            "invalid tts_api_style '{other}' (valid: 'openai_audio', 'elevenlabs')"
                        ))
                    }
                };

                let id = update
                    .id
                    .filter(|id| !id.trim().is_empty())
                    .or_else(|| {
                        config
                            .voice_providers
                            .iter()
                            .find(|e| e.slug == slug)
                            .map(|e| e.id.clone())
                    })
                    .unwrap_or_else(|| generate_voice_provider_id(&slug));

                let label = update.label.unwrap_or_else(|| slug.clone());

                let endpoint = update.endpoint.unwrap_or_default();

                new_entries.push(VoiceProviderCreds {
                    id,
                    slug,
                    label,
                    endpoint,
                    auth_style,
                    capability,
                    stt_api_style,
                    tts_api_style,
                    default_stt_model: update.default_stt_model,
                    default_tts_voice: update.default_tts_voice,
                });
            }
            config.voice_providers = new_entries;
        }

        if let Some(stt) = p.stt_provider {
            let trimmed = stt.trim();
            if !trimmed.is_empty() {
                validate_stt_provider(trimmed)?;
                config.stt_provider = Some(trimmed.to_string());
                // Sync to legacy field so voice_status / voice_stt_dispatch
                // pick up the change without waiting for a restart.
                config.local_ai.stt_provider = trimmed.to_string();
                sync_legacy_stt_engine(&mut config, trimmed);
            }
        }

        if let Some(tts) = p.tts_provider {
            let trimmed = tts.trim();
            if !trimmed.is_empty() {
                validate_tts_provider(trimmed)?;
                config.tts_provider = Some(trimmed.to_string());
                config.local_ai.tts_provider = trimmed.to_string();
            }
        }

        config
            .save()
            .await
            .map_err(crate::config::ops::config_save_error)?;

        log::debug!(
            "[voice-factory] persisted voice provider settings: {} providers, stt={:?}, tts={:?}",
            config.voice_providers.len(),
            config.stt_provider,
            config.tts_provider
        );

        let providers_json: Vec<Value> = config
            .voice_providers
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id,
                    "slug": p.slug,
                    "label": p.label,
                    "endpoint": p.endpoint,
                    "auth_style": p.auth_style.as_str(),
                    "capability": p.capability.as_str(),
                    "stt_api_style": serde_json::to_value(p.stt_api_style).unwrap_or_default(),
                    "tts_api_style": serde_json::to_value(p.tts_api_style).unwrap_or_default(),
                    "default_stt_model": p.default_stt_model,
                    "default_tts_voice": p.default_tts_voice,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "voice_providers": providers_json,
            "stt_provider": config.stt_provider,
            "tts_provider": config.tts_provider,
        }))
    })
}

/// Keep the persisted engine fallback aligned when a known provider is chosen.
/// Custom `slug:model` routing remains in `stt_provider` and leaves the legacy
/// fallback untouched.
fn sync_legacy_stt_engine(config: &mut crate::config::Config, provider: &str) {
    if let Some(engine) = crate::config::SttEngine::parse(provider) {
        config.voice_server.stt_engine = engine;
    }
}

pub(crate) fn voice_list_models_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(
        super::super::registry::voice_schemas("voice_list_models"),
        deserialize_params::<VoiceListModelsParams>,
        run_voice_list_models,
    )
}

pub(crate) fn handle_voice_list_models(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    voice_list_models_controller().invoke(ctx, params)
}

fn run_voice_list_models(ctx: DispatchCtx, p: VoiceListModelsParams) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let provider_id = p.provider_id.trim();
        let capability = p.capability.as_deref().unwrap_or("both");

        log::debug!(
            "[voice-factory] voice_list_models provider_id={provider_id} capability={capability}"
        );

        let entry = config
            .voice_providers
            .iter()
            .find(|e| e.id == provider_id || e.slug == provider_id);

        let models: Vec<Value> = match entry.map(|e| e.slug.as_str()) {
            Some("deepgram") if capability != "tts" => {
                vec![
                    serde_json::json!({"id": "nova-2", "label": "Nova-2 (recommended)"}),
                    serde_json::json!({"id": "nova-2-general", "label": "Nova-2 General"}),
                    serde_json::json!({"id": "nova-2-meeting", "label": "Nova-2 Meeting"}),
                    serde_json::json!({"id": "nova-2-phonecall", "label": "Nova-2 Phone Call"}),
                    serde_json::json!({"id": "enhanced", "label": "Enhanced"}),
                    serde_json::json!({"id": "base", "label": "Base"}),
                ]
            }
            Some("openai") if capability == "stt" => {
                vec![serde_json::json!({"id": "whisper-1", "label": "Whisper v1"})]
            }
            Some("openai") if capability == "tts" => {
                vec![
                    serde_json::json!({"id": "alloy", "label": "Alloy"}),
                    serde_json::json!({"id": "echo", "label": "Echo"}),
                    serde_json::json!({"id": "fable", "label": "Fable"}),
                    serde_json::json!({"id": "onyx", "label": "Onyx"}),
                    serde_json::json!({"id": "nova", "label": "Nova"}),
                    serde_json::json!({"id": "shimmer", "label": "Shimmer"}),
                ]
            }
            Some("openai") => {
                let mut models =
                    vec![serde_json::json!({"id": "whisper-1", "label": "Whisper v1 (STT)"})];
                models.extend([
                    serde_json::json!({"id": "alloy", "label": "Alloy (TTS)"}),
                    serde_json::json!({"id": "echo", "label": "Echo (TTS)"}),
                    serde_json::json!({"id": "fable", "label": "Fable (TTS)"}),
                    serde_json::json!({"id": "onyx", "label": "Onyx (TTS)"}),
                    serde_json::json!({"id": "nova", "label": "Nova (TTS)"}),
                    serde_json::json!({"id": "shimmer", "label": "Shimmer (TTS)"}),
                ]);
                models
            }
            Some("elevenlabs") if capability != "stt" => {
                // ElevenLabs voices require an API call; return empty and let
                // the frontend fetch from /voices if a key is configured.
                vec![]
            }
            _ => vec![],
        };

        Ok(serde_json::json!({ "models": models }))
    })
}

pub(crate) fn voice_test_provider_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(
        super::super::registry::voice_schemas("voice_test_provider"),
        deserialize_params::<VoiceTestProviderParams>,
        run_voice_test_provider,
    )
}

pub(crate) fn handle_voice_test_provider(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    voice_test_provider_controller().invoke(ctx, params)
}

fn run_voice_test_provider(ctx: DispatchCtx, p: VoiceTestProviderParams) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let start = std::time::Instant::now();

        log::debug!(
            "[voice-factory] voice_test_provider workload={} provider={} validate_only={} \
             candidate_key={}",
            p.workload,
            p.provider,
            p.validate_only,
            p.api_key.is_some()
        );

        if !matches!(p.workload.as_str(), "stt" | "tts") {
            return Err(format!(
                "invalid workload '{}' (valid: 'stt', 'tts')",
                p.workload
            ));
        }

        let trimmed = p.provider.trim();

        // `validate_only` is a **dry run** and is honoured for both workloads.
        // It used to be checked inside the "tts" arm alone, which made it a
        // silent no-op for STT — and every provider the settings modal can
        // reach maps to the "stt" workload, so the modal's dry-run request was
        // always answered with a live transcription (#5896).
        //
        // Reserved slugs (`cloud`/`openhuman`/`backend`/`piper`/`whisper`/
        // `local`/empty) are managed or local engines with no user-held API
        // key, so there is nothing to validate. They must still return here
        // rather than fall through: `validate_only` promises no inference
        // call, and falling through would quietly bill the caller for a real
        // transcription/synthesis. `ok: true` because nothing is wrong — a
        // managed provider with no key to check is not a failed check.
        if p.validate_only {
            if crate::config::schema::is_voice_slug_reserved(trimmed) {
                return Ok(serde_json::json!({
                    "ok": true,
                    "detail": "No API key to validate — this provider is managed by OpenHuman.",
                    "latency_ms": start.elapsed().as_millis(),
                }));
            }
            return match validate_provider_key(trimmed, &config, p.api_key.as_deref()).await {
                Ok(detail) => {
                    let elapsed = start.elapsed().as_millis();
                    Ok(serde_json::json!({
                        "ok": true,
                        "detail": format!("{detail} ({elapsed}ms)"),
                        "latency_ms": elapsed,
                    }))
                }
                Err(e) => Ok(serde_json::json!({
                    "ok": false,
                    "detail": format!("Key test failed: {e}"),
                })),
            };
        }

        if p.workload == "stt" {
            let provider = crate::voice::create_stt_provider(&p.provider, "", &config)
                .await
                .map_err(|e| e.to_string())?;

            // 0.1s of silence as WAV (16kHz mono 16-bit PCM) so the local
            // The selected engine can transcribe it without an
            // external binary (issue #3425).
            let silent_wav = generate_silent_wav();
            let audio_b64 = {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(&silent_wav)
            };

            return match provider
                .transcribe(&config, &audio_b64, Some("audio/wav"), None, Some("en"))
                .await
            {
                Ok(_outcome) => {
                    let elapsed = start.elapsed().as_millis();
                    Ok(serde_json::json!({
                        "ok": true,
                        "detail": format!("STT test passed ({elapsed}ms)"),
                        "latency_ms": elapsed,
                    }))
                }
                Err(e) => Ok(serde_json::json!({
                    "ok": false,
                    "detail": format!("STT test failed: {e}"),
                })),
            };
        }

        let provider = crate::voice::create_tts_provider(trimmed, "", &config)
            .await
            .map_err(|e| e.to_string())?;
        match provider.synthesize(&config, "Hello", None).await {
            Ok(_outcome) => {
                let elapsed = start.elapsed().as_millis();
                Ok(serde_json::json!({
                    "ok": true,
                    "detail": format!("TTS test passed ({elapsed}ms)"),
                    "latency_ms": elapsed,
                }))
            }
            Err(e) => Ok(serde_json::json!({
                "ok": false,
                "detail": format!("TTS test failed: {e}"),
            })),
        }
    })
}

#[cfg(test)]
#[path = "provider_server_tests.rs"]
mod tests;
