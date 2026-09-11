//! Calling the `tinyvoice` module: the voice primitives, over the bus.
//!
//! Each function here is the host half of one method on
//! `ai.tinyhumans.tinyvoice.Voice`. They exist so the voice domain does not
//! have to know about proxies, base64 framing, or wire error names — a caller
//! asks a question about a transcript or a buffer and gets an answer.
//!
//! # A call costs about 15 microseconds
//!
//! Measured on the real loaded module (`bench_call` in the `tinyvoice` repo):
//! ~15 µs per round trip, against a 20 ms audio frame. A TinyBus module shares
//! this address space — a call is a channel send and a JSON hop, not IPC.
//!
//! So there is no per-call budget to protect and nothing here is too hot to go
//! over the bus, including the VAD, which runs as a
//! [`VadSession`] driven from the always-on capture loop.
//!
//! **What stays on this side is decided by the audio callback, not by cost.**
//! `cpal` delivers on a realtime thread where blocking is a dropout, so the
//! callback converts the sample format and forwards raw interleaved samples;
//! every transform happens in an async worker that calls this module. That is
//! *less* work on the audio thread than the in-process version did, not more.
//!
//! # Failure is not fatal here
//!
//! Every function returns a [`VoiceCallError`] the caller can fall back from,
//! and the callers do. A module that will not load must degrade voice to its
//! pre-module behaviour — deferring to the agent, or skipping a filter — rather
//! than taking dictation down with it. The one thing none of them may do is
//! guess: see [`is_hallucinated`].

use tinyvoice_bus::names::methods;

use super::{host, ops, registry};
use crate::openhuman::config::Config;

/// Registry id of the module these calls go to.
const MODULE_ID: &str = "tinyvoice";

/// Why a voice call did not produce an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceCallError {
    /// The module is not loaded and cannot be: unsupported host, downloads off,
    /// disabled in config, or a load that already failed in this process.
    Unavailable(String),
    /// The call itself failed — a malformed payload, or a refused argument.
    Failed(String),
}

impl std::fmt::Display for VoiceCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) | Self::Failed(message) => f.write_str(message),
        }
    }
}

/// Which hallucination list applies.
///
/// The contract's own type under the name the voice domain has always used for
/// it. It was redeclared here — with a comment saying it had to be, "because
/// this crate does not depend on `tinyvoice`" — and that is no longer true:
/// `tinyvoice-bus` is exactly that dependency, and it costs `serde` and
/// nothing else.
pub use tinyvoice_bus::transcript::Mode as HallucinationMode;

/// The wire value for a screening mode.
///
/// The interface takes the mode as a plain string argument rather than a JSON
/// value, so this reaches the same spelling the contract's `rename_all =
/// "snake_case"` derive produces without a `serde_json` round trip. The match
/// is exhaustive, so a variant added upstream is a compile error here rather
/// than a mode that silently screens as something else.
fn hallucination_mode_wire(mode: HallucinationMode) -> &'static str {
    match mode {
        HallucinationMode::Dictation => "dictation",
        HallucinationMode::Conversation => "conversation",
    }
}

/// A recognised fast-path voice command, or `Unknown`.
///
/// The contract's own type. `Unknown` carries `#[serde(other)]` upstream, so a
/// module newer than this host — which `is_compatible` permits, it only
/// requires the module's minor version to be at least the host's — reports an
/// intent this build has never heard of as `Unknown` and the utterance goes to
/// the agent, rather than failing to decode.
pub use tinyvoice_bus::VoiceIntent;

/// Classify a command transcript into a fast-path intent.
///
/// The transcript should already have had its wake word removed by
/// [`extract_command`].
///
/// # Errors
///
/// [`VoiceCallError`] when the module is unavailable or the call fails. A
/// caller should treat that as [`VoiceIntent::Unknown`] and hand the transcript
/// to the agent — the fast path is an optimisation, and losing it costs a round
/// trip rather than the request.
pub async fn route(config: &Config, transcript: &str) -> Result<VoiceIntent, VoiceCallError> {
    let json: String = call(config, methods::ROUTE, (transcript,)).await?;
    let intent: VoiceIntent = serde_json::from_str(&json)
        .map_err(|e| VoiceCallError::Failed(format!("could not decode intent: {e}")))?;
    Ok(clamped(intent))
}

/// Bring payloads back inside the range the executors assume.
///
/// The module already clamps a spoken volume to `0..=100`, so in practice this
/// changes nothing. It runs anyway because the value is decoded from a wire
/// payload, and `percent` is interpolated straight into an `osascript` command
/// by `voice::always_on::execute_intent`. A value the host never checked
/// reaching a shell command is the shape of bug worth spending three lines to
/// make impossible, rather than one that depends on a remote clamp staying
/// correct.
///
/// It is a free function rather than an inherent method because the type is
/// the contract's now, and this is host policy: the contract describes what a
/// module may say, not what this host is willing to act on.
#[must_use]
fn clamped(intent: VoiceIntent) -> VoiceIntent {
    match intent {
        VoiceIntent::SetVolume { percent } if percent > 100 => {
            VoiceIntent::SetVolume { percent: 100 }
        }
        other => other,
    }
}

/// Apply the wake-word gate, returning the command that followed it.
///
/// `None` means the utterance was not addressed to the agent, or the wake word
/// arrived with nothing after it. Those are the same outcome for a caller, and
/// the module represents both as an empty string.
///
/// # Errors
///
/// [`VoiceCallError`] when the module is unavailable or the call fails.
pub async fn extract_command(
    config: &Config,
    transcript: &str,
    wake_word: &str,
) -> Result<Option<String>, VoiceCallError> {
    let command: String = call(config, methods::EXTRACT_COMMAND, (transcript, wake_word)).await?;
    Ok(if command.is_empty() {
        None
    } else {
        Some(command)
    })
}

/// Whether the wake word appears near the start of a transcript.
///
/// Distinguished from [`extract_command`] so a caller can acknowledge a bare
/// "Hey Tiny", which otherwise reads to the user as a dead microphone.
///
/// # Errors
///
/// [`VoiceCallError`] when the module is unavailable or the call fails.
pub async fn wake_word_present(
    config: &Config,
    transcript: &str,
    wake_word: &str,
) -> Result<bool, VoiceCallError> {
    call(config, methods::WAKE_WORD_PRESENT, (transcript, wake_word)).await
}

/// Whether an STT transcript looks like a hallucination rather than speech.
///
/// # Errors
///
/// [`VoiceCallError`] when the module is unavailable or the call fails.
///
/// **A caller that cannot reach the module must not guess.** Treating an error
/// as "hallucinated" silently deletes real speech; treating it as "clean" lets
/// `[BLANK_AUDIO]` reach the agent as an instruction. Of the two, passing the
/// text through is recoverable and losing it is not, so callers here fall open
/// — and say so at the call site rather than burying it in a default.
pub async fn is_hallucinated(
    config: &Config,
    text: &str,
    mode: HallucinationMode,
) -> Result<bool, VoiceCallError> {
    call(
        config,
        methods::IS_HALLUCINATED,
        (text, hallucination_mode_wire(mode)),
    )
    .await
}

/// Downmix, resample to 16 kHz, optionally silence-gate, and frame as WAV.
///
/// This is the whole capture-side pipeline in one call. Three separate calls
/// would ship the same audio across the bus three times to do work that is
/// microseconds of arithmetic.
///
/// `samples` are interleaved `f32`; `gate_threshold` of zero disables the
/// silence gate.
///
/// # Errors
///
/// [`VoiceCallError`], including a `Failed` when `samples` is not a whole
/// number of frames for `channels`.
pub async fn prepare_capture(
    config: &Config,
    samples: &[f32],
    source_rate: u32,
    channels: u16,
    gate_threshold: f32,
) -> Result<Vec<u8>, VoiceCallError> {
    let encoded = encode_samples(samples);
    let wav: String = call(
        config,
        methods::PREPARE_CAPTURE,
        (encoded, source_rate, channels, gate_threshold),
    )
    .await?;
    decode_audio(&wav)
}

/// Frame mono `f32` samples as a 16-bit PCM WAV file.
///
/// # Errors
///
/// [`VoiceCallError`] when the module is unavailable or the call fails.
pub async fn encode_wav(
    config: &Config,
    samples: &[f32],
    sample_rate: u32,
) -> Result<Vec<u8>, VoiceCallError> {
    let encoded = encode_samples(samples);
    let wav: String = call(config, methods::ENCODE_WAV, (encoded, sample_rate)).await?;
    decode_audio(&wav)
}

/// Tuning for a VAD session.
///
/// The contract's own type. There is no `from_server_config` on it and there
/// should not be: a crate that any host can link cannot know what *this* host
/// persists, so that mapping stays here as [`vad_config_from_server_config`].
pub use tinyvoice_bus::vad::VadConfig;

/// Build VAD tuning from the persisted voice-server config.
///
/// A free function rather than an inherent method because [`VadConfig`] is the
/// contract's type. The unit conversion is the reason this exists at all:
/// OpenHuman persists the utterance ceiling in seconds and the module speaks
/// milliseconds.
#[must_use]
pub fn vad_config_from_server_config(c: &crate::openhuman::config::VoiceServerConfig) -> VadConfig {
    VadConfig {
        onset_threshold: c.vad_onset_threshold,
        hangover_ms: c.vad_hangover_ms,
        min_speech_ms: c.vad_min_speech_ms,
        // Config stores seconds; the module speaks milliseconds. Clamped to at
        // least 1ms so a zero or negative setting cannot make every utterance
        // close on its first frame.
        max_utterance_ms: (c.vad_max_utterance_secs * 1000.0).round().max(1.0) as u32,
    }
}

/// What the segmenter reported, and at which frame.
///
/// The contract splits these in two — [`VadEvent`] is what happened,
/// [`IndexedVadEvent`] pairs it with the frame — where this host used to carry
/// one enum with `frame` repeated in every variant. The JSON is identical
/// either way: `IndexedVadEvent` flattens its event, so the wire still reads
/// `{"frame": 3, "kind": "speech_start"}`.
pub use tinyvoice_bus::vad::{IndexedVadEvent, VadEvent};

/// A live VAD session held by the module.
///
/// Not `Drop`-based: releasing it needs an async bus call, and a `Drop` impl
/// cannot await. Call [`close`](Self::close) when the capture loop stops. A
/// leaked session costs one map entry in the module until the process exits,
/// and the module caps how many can accumulate.
#[derive(Debug, Clone, Copy)]
pub struct VadSession {
    id: u64,
}

impl VadSession {
    /// Open a session with the given tuning.
    ///
    /// # Errors
    ///
    /// [`VoiceCallError`] when the module is unavailable or the config is
    /// rejected.
    pub async fn open(config: &Config, vad: VadConfig) -> Result<Self, VoiceCallError> {
        let json = serde_json::to_string(&vad)
            .map_err(|e| VoiceCallError::Failed(format!("could not encode VAD config: {e}")))?;
        let id: u64 = call(config, methods::VAD_OPEN, (json,)).await?;
        Ok(Self { id })
    }

    /// Push a batch of frame energies and collect whatever the segmenter says.
    ///
    /// Frame indices in the returned events are relative to `energies`.
    ///
    /// # Errors
    ///
    /// [`VoiceCallError`] when the module is unavailable, the session is not
    /// open, or `frame_ms` is zero.
    pub async fn push(
        &self,
        config: &Config,
        frame_ms: u32,
        energies: &[f32],
    ) -> Result<Vec<IndexedVadEvent>, VoiceCallError> {
        let json: String = call(config, methods::VAD_PUSH, (self.id, frame_ms, energies)).await?;
        serde_json::from_str(&json)
            .map_err(|e| VoiceCallError::Failed(format!("could not decode VAD events: {e}")))
    }

    /// Whether the session is currently inside an utterance.
    ///
    /// # Errors
    ///
    /// [`VoiceCallError`] when the module is unavailable or the session is not
    /// open.
    pub async fn is_speaking(&self, config: &Config) -> Result<bool, VoiceCallError> {
        call(config, methods::VAD_IS_SPEAKING, (self.id,)).await
    }

    /// Abort any in-flight utterance without emitting an event.
    ///
    /// The privacy hook: called when the screen locks or capture is revoked, so
    /// a partial utterance is dropped rather than completed and transcribed.
    ///
    /// # Errors
    ///
    /// [`VoiceCallError`] when the module is unavailable or the session is not
    /// open.
    pub async fn reset(&self, config: &Config) -> Result<(), VoiceCallError> {
        call(config, methods::VAD_RESET, (self.id,)).await
    }

    /// Release the session. Closing one that is already gone is not an error.
    ///
    /// # Errors
    ///
    /// [`VoiceCallError`] only when the module itself is unreachable.
    pub async fn close(&self, config: &Config) -> Result<(), VoiceCallError> {
        call(config, methods::VAD_CLOSE, (self.id,)).await
    }
}

/// Downmix and resample a raw capture buffer to 16 kHz mono samples.
///
/// The sibling of [`prepare_capture`] for a live loop, which needs samples to
/// measure and accumulate rather than a finished container.
///
/// # Errors
///
/// [`VoiceCallError`], including a `Failed` when `samples` is not a whole
/// number of frames for `channels`.
pub async fn prepare_frames(
    config: &Config,
    samples: &[f32],
    source_rate: u32,
    channels: u16,
) -> Result<Vec<f32>, VoiceCallError> {
    let encoded: String = call(
        config,
        methods::PREPARE_FRAMES,
        (encode_samples(samples), source_rate, channels),
    )
    .await?;
    decode_samples(&encoded)
}

/// Root-mean-square energy of each fixed-size frame in a buffer.
///
/// # Errors
///
/// [`VoiceCallError`] when the module is unavailable or `frame_len` is zero.
pub async fn frame_energies(
    config: &Config,
    samples: &[f32],
    frame_len: u32,
) -> Result<Vec<f32>, VoiceCallError> {
    call(
        config,
        methods::FRAME_ENERGIES,
        (encode_samples(samples), frame_len),
    )
    .await
}

/// Frame 16-bit PCM samples as a WAV file, without touching the samples.
///
/// Distinct from [`encode_wav`] because a caller holding `i16` should not have
/// to widen to `f32` and let the module narrow back: that round trip is lossy
/// by one LSB for no reason. This path is exact.
///
/// # Errors
///
/// [`VoiceCallError`] when the module is unavailable or the call fails.
pub async fn encode_wav_pcm16(
    config: &Config,
    samples: &[i16],
    sample_rate: u32,
    channels: u16,
) -> Result<Vec<u8>, VoiceCallError> {
    use base64::Engine as _;
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let wav: String = call(
        config,
        methods::ENCODE_WAV_PCM16,
        (encoded, sample_rate, channels),
    )
    .await?;
    decode_audio(&wav)
}

/// Load the voice module if it is not already serving.
///
/// Callers do not have to invoke this — every operation above does it — but a
/// caller that wraps its work in a deadline should, *outside* that deadline. A
/// first use may download and verify an artifact, and charging that against a
/// dictation timeout means the first utterance a user ever speaks is the one
/// that fails.
///
/// # Errors
///
/// The same [`VoiceCallError::Unavailable`] the operations return.
pub async fn ensure_ready(config: &Config) -> Result<(), VoiceCallError> {
    ops::ensure_loaded(config, MODULE_ID)
        .await
        .map_err(VoiceCallError::Unavailable)
}

/// Base64 little-endian `f32`, which is how the interface carries samples.
fn encode_samples(samples: &[f32]) -> String {
    use base64::Engine as _;
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Decode base64 little-endian `f32` samples the module produced.
fn decode_samples(encoded: &str) -> Result<Vec<f32>, VoiceCallError> {
    let bytes = decode_audio(encoded)?;
    if !bytes.len().is_multiple_of(4) {
        return Err(VoiceCallError::Failed(format!(
            "module returned {} bytes, not a whole number of f32 samples",
            bytes.len()
        )));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

/// Decode a base64 audio payload the module produced.
fn decode_audio(encoded: &str) -> Result<Vec<u8>, VoiceCallError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| VoiceCallError::Failed(format!("module returned invalid base64: {e}")))
}

/// Ensure the module is serving, then make one call on it.
async fn call<A, R>(config: &Config, method: &str, args: A) -> Result<R, VoiceCallError>
where
    A: serde::Serialize + Send,
    R: serde::de::DeserializeOwned,
{
    ops::ensure_loaded(config, MODULE_ID)
        .await
        .map_err(VoiceCallError::Unavailable)?;
    let record = registry::find(MODULE_ID)
        .ok_or_else(|| VoiceCallError::Unavailable(format!("unknown module '{MODULE_ID}'")))?;
    let runtime = host::runtime()
        .await
        .map_err(|_| VoiceCallError::Unavailable("the module bus is not running".to_string()))?;
    let proxy = runtime
        .proxy(record.bus_name, record.object_path)
        .map_err(|error| VoiceCallError::Failed(error.to_string()))?;

    proxy
        .call(method, args)
        .await
        .map_err(|error| classify(&error))
}

/// Map a wire error onto the two outcomes a caller distinguishes.
fn classify(error: &tinybus::Error) -> VoiceCallError {
    let message = error.to_string();
    match error.wire_name() {
        // Loaded but not answering: refused, faulted, or gone.
        name if name.contains("ModuleUnavailable") => VoiceCallError::Unavailable(message),
        _ => VoiceCallError::Failed(message),
    }
}

#[cfg(test)]
#[path = "voice_tests.rs"]
mod tests;
