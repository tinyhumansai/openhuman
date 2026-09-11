//! Microphone audio capture using cpal.
//!
//! Records audio from the default input device and produces 16-kHz mono WAV
//! bytes suitable for STT transcription.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SampleRate, StreamConfig};
use log::{debug, error, info, warn};

use crate::openhuman::config::Config;
use tokio::sync::oneshot;

const LOG_PREFIX: &str = "[voice_capture]";

/// Target sample rate for STT (16 kHz mono).
pub(crate) const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Samples per measurement frame — 20 ms at [`TARGET_SAMPLE_RATE`].
///
/// Matches the always-on loop's framing so a "peak RMS" means the same thing
/// in both paths; a different window would make the same audio report a
/// different peak depending on which recorder captured it.
const FRAME_SAMPLES: u32 = TARGET_SAMPLE_RATE / 50;

/// Most raw samples one recording may accumulate.
///
/// Five minutes of 48 kHz stereo — the widest common device format — which is
/// 57.6M `f32`, so the cap is expressed in samples and bounds memory at ~230
/// MiB worst case and ~9.6 MiB for the 16 kHz mono case the streaming path
/// already bounds the same way (`MAX_FULL_AUDIO_SAMPLES`, #1924).
///
/// This bound became load-bearing when the silence gate moved to the module.
/// The gate used to run *inside* the capture callback and drop sustained
/// silence, so an idle microphone added almost nothing; the callback now
/// accumulates every raw interleaved sample, so a recording that is started
/// and never stopped would grow without limit. Gating still happens — just at
/// finalize, which is too late to bound what the callback collected.
const MAX_RAW_SAMPLES: usize = 48_000 * 2 * 60 * 5;

/// RMS below which the module's silence gate drops audio.
///
/// The value the in-process gate used before this moved to the module.
const SILENCE_GATE_THRESHOLD: f32 = 0.002;

/// Result of a completed recording.
#[derive(Debug, Clone)]
pub struct RecordingResult {
    /// WAV-encoded audio bytes (16 kHz, mono, 16-bit PCM).
    pub wav_bytes: Vec<u8>,
    /// Duration of the recording in seconds.
    pub duration_secs: f32,
    /// Number of samples captured.
    pub sample_count: usize,
    /// Peak RMS energy observed during recording.
    /// Used for silence detection — values below ~0.002 indicate no speech.
    pub peak_rms: f32,
}

/// Handle to a recording in progress. Drop or call `stop()` to end recording.
pub struct RecordingHandle {
    stop_flag: Arc<AtomicBool>,
    result_rx: Option<oneshot::Receiver<Result<RawRecording, String>>>,
    /// A finished result to hand back instead of preparing one.
    ///
    /// Test-only. `stop` now ends in module calls, so a test that wants to
    /// exercise what the *pipeline* does with a result — the short-audio and
    /// silence gates in `server` — would otherwise have to stand up a module
    /// to assert something that has nothing to do with audio.
    #[cfg(test)]
    finalized: Option<Result<RecordingResult, String>>,
}

/// What the capture thread produces: the device's own samples, untouched.
///
/// The thread does no signal processing at all now — it converts the sample
/// format and accumulates. Downmixing, resampling, silence gating and WAV
/// framing all happen in [`RecordingHandle::stop`], through the `tinyvoice`
/// module, off the audio thread.
#[derive(Debug)]
pub struct RawRecording {
    /// Interleaved `f32` samples at the device's own rate.
    pub samples: Vec<f32>,
    /// Device sample rate.
    pub source_rate: u32,
    /// Interleaved channel count.
    pub channels: usize,
}

impl RecordingHandle {
    /// Signal the recording to stop, then turn the raw capture into a result.
    ///
    /// Takes `config` because everything after "stop the stream" is a module
    /// call: downmix, resample, silence-gate and frame the audio. Doing that
    /// here rather than on the capture thread is what keeps the audio callback
    /// free of signal processing.
    ///
    /// # Errors
    ///
    /// The capture error if the recording itself failed, or the module error if
    /// the audio cannot be prepared. Both are strings the caller surfaces.
    pub async fn stop(mut self, config: &Config) -> Result<RecordingResult, String> {
        self.stop_flag.store(true, Ordering::SeqCst);
        debug!("{LOG_PREFIX} stop signal sent");

        #[cfg(test)]
        if let Some(finalized) = self.finalized.take() {
            return finalized;
        }

        let raw = match self.result_rx.take() {
            Some(rx) => rx
                .await
                .map_err(|_| "recording task dropped before completing".to_string())??,
            None => return Err("recording already stopped".to_string()),
        };
        finalize(config, &raw).await
    }

    /// A handle whose `stop` yields `result` without touching audio or the bus.
    #[cfg(test)]
    pub(crate) fn from_test_result(result: Result<RecordingResult, String>) -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            result_rx: None,
            finalized: Some(result),
        }
    }
}

/// Turn a raw capture into a WAV plus the metrics the caller gates on.
///
/// Three module calls rather than one, because the caller needs two different
/// views of the same audio: the peak energy is measured on the *ungated*
/// samples — silence detection has to see the silence — while the WAV is the
/// gated version, which is what the STT engine should be billed for.
async fn finalize(config: &Config, raw: &RawRecording) -> Result<RecordingResult, String> {
    use crate::openhuman::modules::voice as tinyvoice;

    let channels = u16::try_from(raw.channels)
        .map_err(|_| format!("implausible channel count: {}", raw.channels))?;

    // Ungated 16 kHz mono, for the energy measurement.
    let mono = tinyvoice::prepare_frames(config, &raw.samples, raw.source_rate, channels)
        .await
        .map_err(|e| format!("could not prepare captured audio: {e}"))?;

    // One frame's worth per measurement, matching the always-on loop's framing.
    let peak_rms = tinyvoice::frame_energies(config, &mono, FRAME_SAMPLES)
        .await
        .map_err(|e| format!("could not measure captured audio: {e}"))?
        .into_iter()
        .fold(0.0f32, f32::max);

    // Gated, framed as WAV — what actually gets uploaded.
    let wav_bytes = tinyvoice::prepare_capture(
        config,
        &raw.samples,
        raw.source_rate,
        channels,
        SILENCE_GATE_THRESHOLD,
    )
    .await
    .map_err(|e| format!("could not encode captured audio: {e}"))?;

    // Derived from the WAV that is actually returned, not from `mono`.
    //
    // `mono` is the UNGATED buffer — it exists to measure peak energy, which
    // has to see the silence. `wav_bytes` is the gated one. Reporting `mono`'s
    // length here would describe audio the caller does not have: `server.rs`
    // gates on `duration_secs` against `min_duration_secs`, so a recording that
    // is mostly silence would claim a long duration and pass a check it should
    // fail. Before the gate moved to the module it ran in the capture callback,
    // so the buffer this was derived from was already gated and the two agreed.
    //
    // 16-bit mono PCM after a 44-byte header, so two bytes per sample.
    let sample_count = wav_bytes.len().saturating_sub(44) / 2;
    let duration_secs = sample_count as f32 / TARGET_SAMPLE_RATE as f32;
    info!(
        "{LOG_PREFIX} recording finalized: {duration_secs:.1}s, {} bytes WAV, peak_rms={peak_rms:.6}",
        wav_bytes.len()
    );

    Ok(RecordingResult {
        wav_bytes,
        duration_secs,
        sample_count,
        peak_rms,
    })
}

/// Start recording from the default microphone.
///
/// Returns a `RecordingHandle` that must be `.stop().await`-ed to get
/// the captured audio. Recording runs on a dedicated OS thread because
/// `cpal::Stream` is `!Send` (it must be created and dropped on the
/// same thread).
pub fn start_recording() -> Result<RecordingHandle, String> {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_clone = stop_flag.clone();
    let (result_tx, result_rx) = oneshot::channel();

    // Use a oneshot to report whether stream setup succeeded.
    let (setup_tx, setup_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);

    std::thread::Builder::new()
        .name("voice-capture".into())
        .spawn(move || {
            // All cpal objects are created and used on this thread.
            let result = record_on_thread(stop_flag_clone, setup_tx);
            let _ = result_tx.send(result);
        })
        .map_err(|e| format!("failed to spawn capture thread: {e}"))?;

    // Wait for the stream to be set up (or an error).
    match setup_rx.recv() {
        Ok(Ok(())) => {
            info!("{LOG_PREFIX} recording started");
            Ok(RecordingHandle {
                stop_flag,
                result_rx: Some(result_rx),
                #[cfg(test)]
                finalized: None,
            })
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err("capture thread exited before signalling readiness".to_string()),
    }
}

/// Append `samples` to `buffer`, stopping at [`MAX_RAW_SAMPLES`].
///
/// Truncates rather than dropping the whole chunk, so a recording that reaches
/// the cap keeps its first five minutes instead of losing the chunk that
/// crossed the line. Silent about it by design: this runs in the audio callback,
/// where logging on every chunk past the cap would be its own problem.
fn append_capped(buffer: &parking_lot::Mutex<Vec<f32>>, samples: &[f32]) {
    let mut guard = buffer.lock();
    let remaining = MAX_RAW_SAMPLES.saturating_sub(guard.len());
    if remaining == 0 {
        return;
    }
    guard.extend_from_slice(&samples[..samples.len().min(remaining)]);
}

/// Runs the entire recording lifecycle on a single thread (cpal requirement).
fn record_on_thread(
    stop_flag: Arc<AtomicBool>,
    setup_tx: std::sync::mpsc::SyncSender<Result<(), String>>,
) -> Result<RawRecording, String> {
    // --- Cross-platform microphone permission pre-check ---
    use crate::openhuman::desktop::accessibility::{
        detect_microphone_permission, microphone_denied_message, request_microphone_access,
        PermissionState,
    };

    let mic_perm = detect_microphone_permission();
    debug!("{LOG_PREFIX} microphone permission state: {mic_perm:?}");

    match mic_perm {
        PermissionState::Unknown => {
            info!("{LOG_PREFIX} microphone permission not yet determined — requesting access");
            request_microphone_access();
            // Re-check after request (macOS may have shown a prompt).
            let updated = detect_microphone_permission();
            debug!("{LOG_PREFIX} microphone permission after request: {updated:?}");
            if matches!(updated, PermissionState::Denied | PermissionState::Unknown) {
                let msg = microphone_denied_message();
                warn!("{LOG_PREFIX} {msg}");
                let _ = setup_tx.send(Err(msg.clone()));
                return Err(msg);
            }
        }
        PermissionState::Denied => {
            let msg = microphone_denied_message();
            warn!("{LOG_PREFIX} {msg}");
            let _ = setup_tx.send(Err(msg.clone()));
            return Err(msg);
        }
        _ => {} // Granted or Unsupported — proceed normally.
    }

    let host = cpal::default_host();
    let device = match host.default_input_device() {
        Some(d) => d,
        None => {
            // Forward via setup_tx so `start_recording`'s caller sees the
            // real reason instead of the generic "capture thread exited
            // before signalling readiness" fallback that fires when
            // setup_tx is dropped (OPENHUMAN-TAURI-AE). Without this, the
            // user — and Sentry — gets no signal about *which* audio
            // failure occurred.
            let msg = "no default audio input device found".to_string();
            warn!("{LOG_PREFIX} {msg}");
            let _ = setup_tx.send(Err(msg.clone()));
            return Err(msg);
        }
    };

    let device_name = device.name().unwrap_or_else(|_| "<unknown>".into());
    info!("{LOG_PREFIX} using input device: {device_name}");

    let config = match device.supported_input_configs() {
        Ok(supported) => match find_best_config(supported) {
            Ok(cfg) => cfg,
            Err(e) => {
                warn!("{LOG_PREFIX} find_best_config failed ({e}), falling back to default");
                match device.default_input_config() {
                    Ok(cfg) => cfg,
                    Err(e2) => {
                        // Replaces a `.expect()` that would have panicked
                        // and dropped setup_tx — see OPENHUMAN-TAURI-AE.
                        let msg = format!(
                            "no default input config available (best-config failed: {e}; default lookup: {e2})"
                        );
                        error!("{LOG_PREFIX} {msg}");
                        let _ = setup_tx.send(Err(msg.clone()));
                        return Err(msg);
                    }
                }
            }
        },
        Err(e) => {
            warn!("{LOG_PREFIX} failed to query input configs ({e}), using default");
            match device.default_input_config() {
                Ok(cfg) => cfg,
                Err(e2) => {
                    // Forward via setup_tx so callers see the real cpal
                    // error rather than the generic dropped-tx fallback
                    // (OPENHUMAN-TAURI-AE).
                    let msg = format!(
                        "no default input config: {e2} (supported-configs query failed: {e})"
                    );
                    error!("{LOG_PREFIX} {msg}");
                    let _ = setup_tx.send(Err(msg.clone()));
                    return Err(msg);
                }
            }
        }
    };
    let source_sample_rate = config.sample_rate().0;
    let source_channels = config.channels() as usize;

    debug!(
        "{LOG_PREFIX} recording config: rate={source_sample_rate} channels={source_channels} format={:?}",
        config.sample_format()
    );

    let samples: Arc<parking_lot::Mutex<Vec<f32>>> = Arc::new(parking_lot::Mutex::new(
        Vec::with_capacity(TARGET_SAMPLE_RATE as usize * 30),
    ));

    let sample_format = config.sample_format();
    let stream_config: StreamConfig = config.into();

    let stream = {
        let samples_writer = samples.clone();
        match sample_format {
            SampleFormat::F32 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        append_capped(&samples_writer, data);
                    },
                    |err| warn!("{LOG_PREFIX} audio stream error: {err}"),
                    None,
                )
                .map_err(|e| format!("failed to build f32 input stream: {e}")),
            SampleFormat::I16 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        append_capped(&samples_writer, &i16_to_f32(data));
                    },
                    |err| warn!("{LOG_PREFIX} audio stream error: {err}"),
                    None,
                )
                .map_err(|e| format!("failed to build i16 input stream: {e}")),
            SampleFormat::U16 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        append_capped(&samples_writer, &u16_to_f32(data));
                    },
                    |err| warn!("{LOG_PREFIX} audio stream error: {err}"),
                    None,
                )
                .map_err(|e| format!("failed to build u16 input stream: {e}")),
            other => Err(format!("unsupported sample format: {other:?}")),
        }
    };

    // If the preferred config failed, retry with the device's default config.
    //
    // The channel count is bound here, not discarded. It used to be (`_source_channels`)
    // because each callback closed over its own `ch` and downmixed in place. Now
    // that downmixing happens at finalize against these values, a fallback stream
    // with a different channel count would otherwise be de-interleaved as if it
    // had the preferred config's — which turns stereo into garbage rather than mono.
    let (stream, source_sample_rate, source_channels) = match stream {
        Ok(s) => (s, source_sample_rate, source_channels),
        Err(ref preferred_err) => {
            warn!(
                "{LOG_PREFIX} preferred config failed ({preferred_err}), retrying with default config"
            );
            match device.default_input_config() {
                Ok(default_cfg) => {
                    let sr = default_cfg.sample_rate().0;
                    let ch = default_cfg.channels() as usize;
                    let fmt = default_cfg.sample_format();
                    info!("{LOG_PREFIX} fallback config: rate={sr} channels={ch} format={fmt:?}");
                    let sc: StreamConfig = default_cfg.into();
                    let sw = samples.clone();
                    let fallback_stream = match fmt {
                        SampleFormat::F32 => device
                            .build_input_stream(
                                &sc,
                                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                                    append_capped(&sw, data);
                                },
                                |err| warn!("{LOG_PREFIX} audio stream error: {err}"),
                                None,
                            )
                            .map_err(|e| format!("fallback f32 stream failed: {e}")),
                        SampleFormat::I16 => device
                            .build_input_stream(
                                &sc,
                                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                                    append_capped(&sw, &i16_to_f32(data));
                                },
                                |err| warn!("{LOG_PREFIX} audio stream error: {err}"),
                                None,
                            )
                            .map_err(|e| format!("fallback i16 stream failed: {e}")),
                        SampleFormat::U16 => device
                            .build_input_stream(
                                &sc,
                                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                                    append_capped(&sw, &u16_to_f32(data));
                                },
                                |err| warn!("{LOG_PREFIX} audio stream error: {err}"),
                                None,
                            )
                            .map_err(|e| format!("fallback u16 stream failed: {e}")),
                        other => Err(format!("unsupported fallback format: {other:?}")),
                    };
                    match fallback_stream {
                        Ok(s) => (s, sr, ch),
                        Err(e2) => {
                            let msg = format!(
                                "both preferred ({preferred_err}) and fallback ({e2}) configs failed"
                            );
                            let _ = setup_tx.send(Err(msg.clone()));
                            return Err(msg);
                        }
                    }
                }
                Err(e2) => {
                    let msg = format!(
                        "preferred config failed ({preferred_err}) and no default available ({e2})"
                    );
                    let _ = setup_tx.send(Err(msg.clone()));
                    return Err(msg);
                }
            }
        }
    };

    if let Err(e) = stream.play() {
        let msg = format!("failed to start audio stream: {e}");
        let _ = setup_tx.send(Err(msg.clone()));
        return Err(msg);
    }

    // Signal success so start_recording() returns.
    let _ = setup_tx.send(Ok(()));

    // Poll stop flag while keeping the stream alive on this thread.
    while !stop_flag.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    debug!("{LOG_PREFIX} stop flag detected, finalizing recording");
    drop(stream);

    let samples = samples.lock().clone();
    if samples.is_empty() {
        warn!("{LOG_PREFIX} no audio samples captured");
        return Err("no audio samples captured".to_string());
    }
    debug!(
        "{LOG_PREFIX} captured {} raw interleaved samples at {source_sample_rate}Hz x{source_channels}",
        samples.len()
    );
    Ok(RawRecording {
        samples,
        source_rate: source_sample_rate,
        channels: source_channels,
    })
}

/// List available input devices.
pub fn list_input_devices() -> Result<Vec<String>, String> {
    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .map_err(|e| format!("failed to enumerate input devices: {e}"))?;

    let names: Vec<String> = devices.filter_map(|d| d.name().ok()).collect();

    debug!("{LOG_PREFIX} found {} input devices", names.len());
    Ok(names)
}

/// Convert interleaved signed `i16` PCM samples to normalised `f32` in
/// `[-1.0, 1.0)` (`s / 32768`). Extracted so the preferred and fallback stream
/// paths share one conversion instead of duplicating it inline.
pub(crate) fn i16_to_f32(data: &[i16]) -> Vec<f32> {
    data.iter().map(|&s| s as f32 / 32768.0).collect()
}

/// Convert interleaved unsigned `u16` PCM samples (mid-scale 32768) to
/// normalised `f32` in `[-1.0, 1.0)` (`(s - 32768) / 32768`). Shared by the
/// preferred and fallback stream paths.
pub(crate) fn u16_to_f32(data: &[u16]) -> Vec<f32> {
    data.iter()
        .map(|&s| (s as f32 - 32768.0) / 32768.0)
        .collect()
}

/// Find the best input config — prefer 16 kHz mono, else closest match.
fn find_best_config(
    configs: impl Iterator<Item = cpal::SupportedStreamConfigRange>,
) -> Result<cpal::SupportedStreamConfig, String> {
    let mut configs_vec: Vec<cpal::SupportedStreamConfigRange> = configs.collect();
    if configs_vec.is_empty() {
        return Err("no supported audio input configurations found".to_string());
    }

    // Sort: prefer configs whose range includes 16kHz, then by fewer channels.
    configs_vec.sort_by(|a, b| {
        let a_has_target = a.min_sample_rate().0 <= TARGET_SAMPLE_RATE
            && a.max_sample_rate().0 >= TARGET_SAMPLE_RATE;
        let b_has_target = b.min_sample_rate().0 <= TARGET_SAMPLE_RATE
            && b.max_sample_rate().0 >= TARGET_SAMPLE_RATE;

        b_has_target
            .cmp(&a_has_target)
            .then(a.channels().cmp(&b.channels()))
    });

    let best = &configs_vec[0];
    let rate = if best.min_sample_rate().0 <= TARGET_SAMPLE_RATE
        && best.max_sample_rate().0 >= TARGET_SAMPLE_RATE
    {
        SampleRate(TARGET_SAMPLE_RATE)
    } else {
        // Use the maximum supported rate and resample later.
        best.max_sample_rate()
    };

    Ok((*best).with_sample_rate(rate))
}

#[cfg(test)]
#[path = "audio_capture_tests.rs"]
mod tests;
