//! Handlers for provider settings, model listing, provider testing, and server lifecycle.

use serde_json::{Map, Value};

use crate::core::all::ControllerFuture;
use crate::openhuman::config::rpc as config_rpc;

use crate::openhuman::voice::schemas::helpers::{
    deserialize_params, generate_silent_wav, validate_stt_provider, validate_tts_provider,
    validate_tts_provider_key,
};
use crate::openhuman::voice::schemas::params::{
    OverlaySttNotifyParams, OverlaySttState, SetProvidersParams, VoiceListModelsParams,
    VoiceTestProviderParams, VoiceUpdateProviderSettingsParams,
};

// ---------------------------------------------------------------------------
// Provider configuration handlers
// ---------------------------------------------------------------------------

pub(crate) fn handle_voice_set_providers(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<SetProvidersParams>(params)?;
        let mut config = config_rpc::load_config_with_timeout().await?;

        if let Some(stt) = p
            .stt_provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            validate_stt_provider(stt)?;
            config.local_ai.stt_provider = stt.to_string();
            config.stt_provider = Some(stt.to_string());
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

        config.save().await.map_err(|e| e.to_string())?;
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

pub(crate) fn handle_voice_update_provider_settings(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        use crate::openhuman::config::schema::voice_providers::{
            generate_voice_provider_id, is_voice_slug_reserved, SttApiStyle, TtsApiStyle,
            VoiceCapability, VoiceProviderCreds,
        };

        let p = deserialize_params::<VoiceUpdateProviderSettingsParams>(params)?;
        let mut config = config_rpc::load_config_with_timeout().await?;

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
                    Some("bearer") | None => crate::openhuman::config::schema::AuthStyle::Bearer,
                    Some("none") => crate::openhuman::config::schema::AuthStyle::None,
                    Some(other) => {
                        return Err(format!(
                        "invalid auth_style '{other}' for voice provider (valid: 'bearer', 'none')"
                    ))
                    }
                };

                let stt_api_style = match update.stt_api_style.as_deref() {
                    Some("deepgram") => SttApiStyle::Deepgram,
                    Some("openai_audio") | None => SttApiStyle::OpenaiAudio,
                    Some(other) => {
                        return Err(format!(
                            "invalid stt_api_style '{other}' (valid: 'openai_audio', 'deepgram')"
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

        config.save().await.map_err(|e| e.to_string())?;

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
                    "stt_api_style": serde_json::to_value(&p.stt_api_style).unwrap_or_default(),
                    "tts_api_style": serde_json::to_value(&p.tts_api_style).unwrap_or_default(),
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

pub(crate) fn handle_voice_list_models(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<VoiceListModelsParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
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

pub(crate) fn handle_voice_test_provider(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<VoiceTestProviderParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        let start = std::time::Instant::now();

        log::debug!(
            "[voice-factory] voice_test_provider workload={} provider={}",
            p.workload,
            p.provider
        );

        match p.workload.as_str() {
            "stt" => {
                let provider =
                    crate::openhuman::voice::create_stt_provider(&p.provider, "", &config)
                        .map_err(|e| e.to_string())?;

                // 0.1s of silence as WAV (8kHz mono 16-bit PCM).
                let silent_wav = generate_silent_wav();
                let audio_b64 = {
                    use base64::Engine;
                    base64::engine::general_purpose::STANDARD.encode(&silent_wav)
                };

                match provider
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
                }
            }
            "tts" => {
                let trimmed = p.provider.trim();
                if p.validate_only && !matches!(trimmed, "cloud" | "openhuman" | "piper" | "") {
                    match validate_tts_provider_key(trimmed, &config).await {
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
                            "detail": format!("TTS test failed: {e}"),
                        })),
                    }
                } else {
                    let provider =
                        crate::openhuman::voice::create_tts_provider(trimmed, "", &config)
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
                }
            }
            other => Err(format!("invalid workload '{other}' (valid: 'stt', 'tts')")),
        }
    })
}

// ---------------------------------------------------------------------------
// Voice server lifecycle handlers
// ---------------------------------------------------------------------------

pub(crate) fn handle_voice_server_start(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        use crate::openhuman::voice::hotkey::ActivationMode;
        use crate::openhuman::voice::server::{global_server, VoiceServerConfig};

        let config = config_rpc::load_config_with_timeout().await?;

        let hotkey = params
            .get("hotkey")
            .and_then(|v| v.as_str())
            .unwrap_or(&config.voice_server.hotkey)
            .to_string();

        let activation_mode = match params.get("activation_mode").and_then(|v| v.as_str()) {
            Some("push") => ActivationMode::Push,
            Some("tap") => ActivationMode::Tap,
            Some(other) => {
                log::warn!(
                    "[voice_server] unrecognized activation_mode '{}', defaulting to Push",
                    other
                );
                ActivationMode::Push
            }
            None => match config.voice_server.activation_mode {
                crate::openhuman::config::VoiceActivationMode::Push => ActivationMode::Push,
                crate::openhuman::config::VoiceActivationMode::Tap => ActivationMode::Tap,
            },
        };

        let skip_cleanup = params
            .get("skip_cleanup")
            .and_then(|v| v.as_bool())
            .unwrap_or(config.voice_server.skip_cleanup);

        let server_config = VoiceServerConfig {
            hotkey,
            activation_mode,
            skip_cleanup,
            context: None,
            min_duration_secs: config.voice_server.min_duration_secs,
            silence_threshold: config.voice_server.silence_threshold,
            custom_dictionary: config.voice_server.custom_dictionary.clone(),
        };

        // Check if a server is already running with a different config.
        if let Some(existing) = crate::openhuman::voice::server::try_global_server() {
            let existing_status = existing.status().await;
            if existing_status.state != crate::openhuman::voice::server::ServerState::Stopped {
                if existing_status.hotkey != server_config.hotkey
                    || existing_status.activation_mode != server_config.activation_mode
                {
                    return Err(format!(
                        "voice server already running (hotkey={}, mode={:?}); \
                         stop it first before starting with different config",
                        existing_status.hotkey, existing_status.activation_mode
                    ));
                }
                // Same config, already running — return current status.
                return serde_json::to_value(existing_status)
                    .map_err(|e| format!("serialize error: {e}"));
            }
        }

        let server = global_server(server_config);
        let config_clone = config.clone();
        let server_for_err = server.clone();

        tokio::spawn(async move {
            if let Err(e) = server.run(&config_clone).await {
                log::error!("[voice_server] server exited with error: {e}");
                server_for_err.set_last_error(&e).await;
            }
        });

        // Give the server a moment to start.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        if let Some(s) = crate::openhuman::voice::server::try_global_server() {
            let status = s.status().await;
            serde_json::to_value(status).map_err(|e| format!("serialize error: {e}"))
        } else {
            Err("voice server failed to initialize".to_string())
        }
    })
}

pub(crate) fn handle_voice_server_stop(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        if let Some(server) = crate::openhuman::voice::server::try_global_server() {
            server.stop().await;
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let status = server.status().await;
            serde_json::to_value(status).map_err(|e| format!("serialize error: {e}"))
        } else {
            // Not running — return a stopped status rather than an error.
            let status = crate::openhuman::voice::server::VoiceServerStatus {
                state: crate::openhuman::voice::server::ServerState::Stopped,
                hotkey: String::new(),
                activation_mode: crate::openhuman::voice::hotkey::ActivationMode::Push,
                transcription_count: 0,
                last_error: None,
            };
            serde_json::to_value(status).map_err(|e| format!("serialize error: {e}"))
        }
    })
}

pub(crate) fn handle_voice_server_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        if let Some(server) = crate::openhuman::voice::server::try_global_server() {
            let status = server.status().await;
            serde_json::to_value(status).map_err(|e| format!("serialize error: {e}"))
        } else {
            let status = crate::openhuman::voice::server::VoiceServerStatus {
                state: crate::openhuman::voice::server::ServerState::Stopped,
                hotkey: String::new(),
                activation_mode: crate::openhuman::voice::hotkey::ActivationMode::Push,
                transcription_count: 0,
                last_error: None,
            };
            serde_json::to_value(status).map_err(|e| format!("serialize error: {e}"))
        }
    })
}

// ---------------------------------------------------------------------------
// Overlay STT notify handler
// ---------------------------------------------------------------------------

pub(crate) fn handle_overlay_stt_notify(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<OverlaySttNotifyParams>(params)?;
        log::debug!(
            "[overlay_stt_notify] state={:?}, has_text={}, text_len={}",
            p.state,
            p.text.is_some(),
            p.text.as_deref().map_or(0, |t| t.len())
        );

        use crate::openhuman::voice::dictation_listener::{
            publish_dictation_event, publish_transcription, DictationEvent,
        };

        match p.state {
            OverlaySttState::RecordingStarted => {
                publish_dictation_event(DictationEvent {
                    event_type: "pressed".to_string(),
                    hotkey: "chat_button".to_string(),
                    activation_mode: "toggle".to_string(),
                });
            }
            OverlaySttState::TranscriptionDone => {
                let text = p.text.ok_or_else(|| {
                    "invalid params: `text` is required for transcription_done".to_string()
                })?;
                publish_transcription(text);
                publish_dictation_event(DictationEvent {
                    event_type: "released".to_string(),
                    hotkey: "chat_button".to_string(),
                    activation_mode: "toggle".to_string(),
                });
            }
            OverlaySttState::Cancelled | OverlaySttState::Error => {
                publish_dictation_event(DictationEvent {
                    event_type: "released".to_string(),
                    hotkey: "chat_button".to_string(),
                    activation_mode: "toggle".to_string(),
                });
            }
        }

        Ok(serde_json::json!({ "ok": true }))
    })
}
