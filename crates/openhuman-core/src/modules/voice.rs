//! TinyVoice adapter: transcript screening and PCM16 WAV framing.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use std::time::Duration;
use tinyvoice_bus::names::methods;

use super::{ops, registry};
use crate::config::Config;

const MODULE_ID: &str = "tinyvoice";

/// What a bounded caller reports when TinyVoice has not finished loading.
pub const STILL_LOADING_MESSAGE: &str =
    "the voice module is still downloading or loading; try again in a moment";

/// Why a voice screening call did not produce an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceCallError {
    Unavailable(String),
    Failed(String),
}

impl std::fmt::Display for VoiceCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) | Self::Failed(message) => f.write_str(message),
        }
    }
}

/// Hallucination list used for a transcript check.
pub use tinyvoice_bus::transcript::Mode as HallucinationMode;

/// Whether an STT transcript looks like a hallucination rather than speech.
///
/// A caller that cannot reach TinyVoice must pass the transcript through: an
/// unavailable filter must not silently discard real speech.
pub async fn is_hallucinated(
    config: &Config,
    text: &str,
    mode: HallucinationMode,
) -> Result<bool, VoiceCallError> {
    let mode = match mode {
        HallucinationMode::Dictation => "dictation",
        HallucinationMode::Conversation => "conversation",
    };
    call(config, methods::IS_HALLUCINATED, (text, mode), None).await
}

/// Wrap 16-bit PCM samples in a canonical 44-byte WAV container.
///
/// The samples cross the bus as their little-endian bytes, so the framing is
/// exact: no `f32` round trip touches them (TinyVoice's `EncodeWavPcm16`).
///
/// `load_within` bounds how long this caller waits for the lazily loaded
/// module (`None` waits without bound). When the bound passes first the call
/// fails with [`VoiceCallError::Unavailable`] and the download keeps running.
pub async fn encode_wav_pcm16(
    config: &Config,
    samples: &[i16],
    sample_rate: u32,
    channels: u16,
    load_within: Option<Duration>,
) -> Result<Vec<u8>, VoiceCallError> {
    tracing::debug!(
        method = methods::ENCODE_WAV_PCM16,
        sample_count = samples.len(),
        sample_rate,
        channels,
        load_wait_ms = load_within.map(|wait| wait.as_millis() as u64),
        "[voice] encoding PCM16 WAV"
    );
    let wav: String = call(
        config,
        methods::ENCODE_WAV_PCM16,
        (pcm16_le_base64(samples), sample_rate, channels),
        load_within,
    )
    .await?;
    let decoded = BASE64
        .decode(wav)
        .map_err(|error| VoiceCallError::Failed(format!("invalid WAV payload: {error}")));
    if let Err(error) = &decoded {
        tracing::debug!(method = methods::ENCODE_WAV_PCM16, %error, "[voice] invalid WAV response");
    }
    decoded
}

/// Base64 of `samples` as little-endian bytes, the `EncodeWavPcm16` input.
fn pcm16_le_base64(samples: &[i16]) -> String {
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    BASE64.encode(bytes)
}

async fn call<A, R>(
    config: &Config,
    method: &str,
    args: A,
    load_within: Option<Duration>,
) -> Result<R, VoiceCallError>
where
    A: serde::Serialize + Send,
    R: serde::de::DeserializeOwned,
{
    let load_started = std::time::Instant::now();
    ops::ensure_loaded_within(config, MODULE_ID, load_within)
        .await
        .map_err(|error| {
            tracing::debug!(method, load_wait_ms = load_started.elapsed().as_millis() as u64, %error, "[voice] module load failed");
            match error {
            tinybus::module::LoadError::StillLoading => {
                VoiceCallError::Unavailable(STILL_LOADING_MESSAGE.to_string())
            }
            other => VoiceCallError::Unavailable(ops::load_error_message(other)),
            }
        })?;
    let record = registry::find(MODULE_ID)
        .ok_or_else(|| VoiceCallError::Unavailable(format!("unknown module '{MODULE_ID}'")))?;
    let runtime = super::runtime::runtime()
        .await
        .map_err(|_| VoiceCallError::Unavailable("the module bus is not running".to_string()))?;
    let proxy = runtime
        .proxy(record.bus_name, record.object_path)
        .map_err(|error| VoiceCallError::Failed(error.to_string()))?;
    proxy.call(method, args).await.map_err(|error| {
        tracing::debug!(method, %error, "[voice] module bus call failed");
        match error.wire_name() {
            name if name.contains("ModuleUnavailable") => {
                VoiceCallError::Unavailable(error.to_string())
            }
            _ => VoiceCallError::Failed(error.to_string()),
        }
    })
}

#[cfg(test)]
#[path = "voice_tests.rs"]
mod tests;
