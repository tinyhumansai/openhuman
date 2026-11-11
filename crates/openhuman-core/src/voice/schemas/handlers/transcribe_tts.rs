//! Handlers for transcription, TTS synthesis, and factory-dispatch RPCs.

use crate::config::Config;
use crate::core::all::{BoundRegisteredController, ControllerFuture, ControllerTask};
use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::voice::schemas::helpers::{
    deserialize_params, effective_stt_provider, effective_tts_provider, to_json,
};
use crate::voice::schemas::params::{
    ReplySynthesizeParams, SttDispatchParams, TranscribeBytesParams, TranscribeParams,
    TtsDispatchParams, TtsParams,
};

fn load_voice_config(ctx: DispatchCtx) -> ControllerTask<Config> {
    Box::pin(async move { ctx.load_config().await })
}

fn ignored_params(_params: Map<String, Value>) -> Result<(), String> {
    Ok(())
}

pub(crate) fn voice_status_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(
        super::super::registry::voice_schemas("voice_status"),
        load_voice_config,
        ignored_params,
        run_voice_status,
    )
}

pub(crate) fn voice_transcribe_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(
        super::super::registry::voice_schemas("voice_transcribe"),
        load_voice_config,
        deserialize_params::<TranscribeParams>,
        run_voice_transcribe,
    )
}

pub(crate) fn voice_transcribe_bytes_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(
        super::super::registry::voice_schemas("voice_transcribe_bytes"),
        load_voice_config,
        deserialize_params::<TranscribeBytesParams>,
        run_voice_transcribe_bytes,
    )
}

pub(crate) fn voice_tts_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(
        super::super::registry::voice_schemas("voice_tts"),
        load_voice_config,
        deserialize_params::<TtsParams>,
        run_voice_tts,
    )
}

pub(crate) fn voice_reply_synthesize_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(
        super::super::registry::voice_schemas("voice_reply_synthesize"),
        load_voice_config,
        deserialize_params::<ReplySynthesizeParams>,
        run_voice_reply_synthesize,
    )
}

pub(crate) fn voice_stt_dispatch_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(
        super::super::registry::voice_schemas("voice_stt_dispatch"),
        load_voice_config,
        deserialize_params::<SttDispatchParams>,
        run_voice_stt_dispatch,
    )
}

pub(crate) fn voice_tts_dispatch_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(
        super::super::registry::voice_schemas("voice_tts_dispatch"),
        load_voice_config,
        deserialize_params::<TtsDispatchParams>,
        run_voice_tts_dispatch,
    )
}

fn run_voice_status(_ctx: DispatchCtx, config: Config, (): ()) -> ControllerFuture {
    Box::pin(async move { to_json(crate::voice::voice_status(&config).await?) })
}

fn run_voice_transcribe(
    _ctx: DispatchCtx,
    config: Config,
    p: TranscribeParams,
) -> ControllerFuture {
    Box::pin(async move {
        to_json(
            crate::voice::voice_transcribe(
                &config,
                &p.audio_path,
                p.context.as_deref(),
                p.skip_cleanup,
            )
            .await?,
        )
    })
}

fn run_voice_transcribe_bytes(
    _ctx: DispatchCtx,
    config: Config,
    p: TranscribeBytesParams,
) -> ControllerFuture {
    Box::pin(async move {
        to_json(
            crate::voice::voice_transcribe_bytes(
                &config,
                &p.audio_bytes,
                p.extension,
                p.context.as_deref(),
                p.skip_cleanup,
            )
            .await?,
        )
    })
}

fn run_voice_tts(_ctx: DispatchCtx, config: Config, p: TtsParams) -> ControllerFuture {
    Box::pin(async move {
        to_json(crate::voice::voice_tts(&config, &p.text, p.output_path.as_deref()).await?)
    })
}

fn run_voice_reply_synthesize(
    _ctx: DispatchCtx,
    config: Config,
    p: ReplySynthesizeParams,
) -> ControllerFuture {
    Box::pin(async move {
        // Dispatch through the TTS factory so the user's `tts_provider`
        // setting (cloud / piper / …) is honored on the spoken-reply path,
        // not just the dedicated `voice_tts_dispatch` RPC. Without this
        // routing, the settings dropdown was effectively decorative —
        // selecting "piper" persisted to config but conversation replies
        // still hit the cloud TTS proxy.
        let provider_name = effective_tts_provider(&config);
        // Only default to the Piper voice id when the active provider is
        // actually Piper. Passing a Piper voice id to a cloud TTS provider
        // would send an invalid voice to the upstream API.
        let voice = p
            .voice_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if provider_name == "piper" {
                    crate::voice::DEFAULT_PIPER_VOICE.to_string()
                } else {
                    String::new()
                }
            });
        let effective_voice = if voice.is_empty() {
            None
        } else {
            Some(voice.as_str())
        };
        log::debug!(
            "[voice-factory] voice_reply_synthesize dispatch provider={provider_name} voice={voice}"
        );
        let provider = crate::voice::create_tts_provider(&provider_name, &voice, &config)
            .await
            .map_err(|e| e.to_string())?;
        to_json(
            provider
                .synthesize(&config, &p.text, effective_voice)
                .await?,
        )
    })
}

fn run_voice_stt_dispatch(
    _ctx: DispatchCtx,
    config: Config,
    p: SttDispatchParams,
) -> ControllerFuture {
    Box::pin(async move {
        let provider_name = p
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| effective_stt_provider(&config));
        let model = p
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_default();

        log::debug!(
            "[voice-factory] RPC voice_stt_dispatch provider={provider_name} model={model}"
        );
        let provider = crate::voice::create_stt_provider(&provider_name, &model, &config)
            .await
            .map_err(|e| e.to_string())?;
        let outcome = provider
            .transcribe(
                &config,
                &p.audio_base64,
                p.mime_type.as_deref(),
                p.file_name.as_deref(),
                p.language.as_deref(),
            )
            .await?;
        let value = serde_json::json!({
            "text": outcome.value.text,
            "provider": outcome.value.provider,
        });
        Ok(value)
    })
}

fn run_voice_tts_dispatch(
    _ctx: DispatchCtx,
    config: Config,
    p: TtsDispatchParams,
) -> ControllerFuture {
    Box::pin(async move {
        let provider_name = p
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| effective_tts_provider(&config));
        // Only fall back to the Piper default voice id when the provider is
        // Piper; sending a Piper voice id to a cloud TTS endpoint is invalid.
        let voice = p
            .voice
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if provider_name == "piper" {
                    crate::voice::DEFAULT_PIPER_VOICE.to_string()
                } else {
                    String::new()
                }
            });
        let effective_voice = if voice.is_empty() {
            None
        } else {
            Some(voice.as_str())
        };

        log::debug!(
            "[voice-factory] RPC voice_tts_dispatch provider={provider_name} voice={voice}"
        );
        let provider = crate::voice::create_tts_provider(&provider_name, &voice, &config)
            .await
            .map_err(|e| e.to_string())?;
        let outcome = provider
            .synthesize(&config, &p.text, effective_voice)
            .await?;
        to_json(outcome)
    })
}

pub(crate) fn handle_voice_status(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    voice_status_controller().invoke(ctx, params)
}

pub(crate) fn handle_voice_transcribe(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    voice_transcribe_controller().invoke(ctx, params)
}

pub(crate) fn handle_voice_transcribe_bytes(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    voice_transcribe_bytes_controller().invoke(ctx, params)
}

pub(crate) fn handle_voice_tts(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    voice_tts_controller().invoke(ctx, params)
}

pub(crate) fn handle_voice_reply_synthesize(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    voice_reply_synthesize_controller().invoke(ctx, params)
}

pub(crate) fn handle_voice_stt_dispatch(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    voice_stt_dispatch_controller().invoke(ctx, params)
}

pub(crate) fn handle_voice_tts_dispatch(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    voice_tts_dispatch_controller().invoke(ctx, params)
}
