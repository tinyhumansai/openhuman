//! Shared helper utilities for voice controller schemas.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::{FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

pub(super) fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    let json_val =
        serde_json::to_value(outcome.value).map_err(|e| format!("serialize error: {e}"))?;
    Ok(json_val)
}

pub(super) fn deserialize_params<T: DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

pub(super) fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

pub(super) fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

pub(super) fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment,
        required: false,
    }
}

pub(super) fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

pub(super) fn validate_stt_provider(provider: &str) -> Result<(), String> {
    match provider {
        "cloud" | "openhuman" | "whisper" => Ok(()),
        other => {
            // Accept slug:model grammar or bare slug — the factory will
            // validate against the voice_providers registry at dispatch time.
            if other.contains(':') || !other.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "invalid stt_provider '{other}' (valid: 'cloud', 'whisper', or '<slug>:<model>')"
                ))
            }
        }
    }
}

pub(super) fn validate_tts_provider(provider: &str) -> Result<(), String> {
    match provider {
        "cloud" | "openhuman" | "piper" => Ok(()),
        other => {
            if other.contains(':') || !other.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "invalid tts_provider '{other}' (valid: 'cloud', 'piper', or '<slug>:<voice>')"
                ))
            }
        }
    }
}

pub(super) fn effective_stt_provider(config: &crate::openhuman::config::Config) -> String {
    crate::openhuman::voice::effective_stt_provider(config)
}

pub(super) fn effective_tts_provider(config: &crate::openhuman::config::Config) -> String {
    crate::openhuman::voice::effective_tts_provider(config)
}

/// Validate a TTS provider's API key by hitting a lightweight read-only endpoint
/// rather than synthesizing audio (which requires a valid voice ID).
pub(super) async fn validate_tts_provider_key(
    provider: &str,
    config: &crate::openhuman::config::Config,
) -> Result<String, String> {
    let (slug, _model) = if let Some(pos) = provider.find(':') {
        (&provider[..pos], &provider[pos + 1..])
    } else {
        (provider, "")
    };

    let entry = config
        .voice_providers
        .iter()
        .find(|p| p.slug == slug)
        .ok_or_else(|| format!("no voice provider with slug '{slug}'"))?;

    let api_key = crate::openhuman::inference::provider::factory::lookup_key_for_slug(slug, config)
        .unwrap_or_default();

    if api_key.is_empty() {
        return Err("no API key configured for this provider".to_string());
    }

    let endpoint = entry.endpoint.trim_end_matches('/');
    let client = reqwest::Client::new();

    // ElevenLabs: GET /user/subscription requires only basic auth (no
    // extra scopes like voices_read). OpenAI / generic: GET /models.
    let url = if slug == "elevenlabs" {
        format!("{endpoint}/user/subscription")
    } else {
        format!("{endpoint}/models")
    };

    let mut req = client.get(&url);
    if slug == "elevenlabs" {
        req = req.header("xi-api-key", &api_key);
    } else {
        req = req.header("Authorization", format!("Bearer {api_key}"));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if resp.status().is_success() {
        Ok("TTS provider key is valid".to_string())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("API returned {status}: {body}"))
    }
}

/// Generate a minimal WAV file with ~0.1s of silence (8kHz mono 16-bit PCM).
pub(super) fn generate_silent_wav() -> Vec<u8> {
    let sample_rate: u32 = 8000;
    let num_samples: u32 = 800; // 0.1s
    let bits_per_sample: u16 = 16;
    let num_channels: u16 = 1;
    let byte_rate = sample_rate * u32::from(num_channels) * u32::from(bits_per_sample) / 8;
    let block_align = num_channels * bits_per_sample / 8;
    let data_size = num_samples * u32::from(block_align);
    let file_size = 36 + data_size;

    let mut wav = Vec::with_capacity(44 + data_size as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // subchunk1 size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&num_channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.extend(std::iter::repeat(0u8).take(data_size as usize));
    wav
}
