//! Handlers for transcription, TTS synthesis, and factory-dispatch RPCs.

use serde_json::{Map, Value};

use crate::core::all::ControllerFuture;
use crate::openhuman::config::rpc as config_rpc;

use crate::openhuman::voice::schemas::helpers::{
    deserialize_params, effective_stt_provider, effective_tts_provider, to_json,
};
use crate::openhuman::voice::schemas::params::{
    CloudTranscribeParams, ReplySynthesizeParams, SttDispatchParams, TranscribeBytesParams,
    TranscribeParams, TtsDispatchParams, TtsParams,
};

pub(crate) fn handle_voice_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::voice::voice_status(&config).await?)
    })
}

pub(crate) fn handle_voice_transcribe(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<TranscribeParams>(params)?;
        to_json(
            crate::openhuman::voice::voice_transcribe(
                &config,
                &p.audio_path,
                p.context.as_deref(),
                p.skip_cleanup,
            )
            .await?,
        )
    })
}

pub(crate) fn handle_voice_transcribe_bytes(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<TranscribeBytesParams>(params)?;
        to_json(
            crate::openhuman::voice::voice_transcribe_bytes(
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

pub(crate) fn handle_voice_tts(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<TtsParams>(params)?;
        to_json(
            crate::openhuman::voice::voice_tts(&config, &p.text, p.output_path.as_deref()).await?,
        )
    })
}

pub(crate) fn handle_voice_reply_synthesize(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<ReplySynthesizeParams>(params)?;
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
                    crate::openhuman::voice::DEFAULT_PIPER_VOICE.to_string()
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
        let provider =
            crate::openhuman::voice::create_tts_provider(&provider_name, &voice, &config)
                .map_err(|e| e.to_string())?;
        to_json(
            provider
                .synthesize(&config, &p.text, effective_voice)
                .await?,
        )
    })
}

pub(crate) fn handle_voice_cloud_transcribe(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<CloudTranscribeParams>(params)?;
        let opts = crate::openhuman::voice::cloud_transcribe::CloudTranscribeOptions {
            model: p.model,
            language: p.language,
            mime_type: p.mime_type,
            file_name: p.file_name,
        };
        to_json(
            crate::openhuman::voice::cloud_transcribe::transcribe_cloud(
                &config,
                &p.audio_base64,
                &opts,
            )
            .await?,
        )
    })
}

pub(crate) fn handle_voice_stt_dispatch(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<SttDispatchParams>(params)?;
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
            .unwrap_or_else(|| crate::openhuman::voice::DEFAULT_WHISPER_MODEL.to_string());

        log::debug!(
            "[voice-factory] RPC voice_stt_dispatch provider={provider_name} model={model}"
        );
        let provider =
            crate::openhuman::voice::create_stt_provider(&provider_name, &model, &config)
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

pub(crate) fn handle_voice_tts_dispatch(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<TtsDispatchParams>(params)?;
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
                    crate::openhuman::voice::DEFAULT_PIPER_VOICE.to_string()
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
        let provider =
            crate::openhuman::voice::create_tts_provider(&provider_name, &voice, &config)
                .map_err(|e| e.to_string())?;
        let outcome = provider
            .synthesize(&config, &p.text, effective_voice)
            .await?;
        to_json(outcome)
    })
}
