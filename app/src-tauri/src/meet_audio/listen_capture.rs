//! Capture the embedded Meet webview's audio output and forward it to
//! the core meet_agent loop.
//!
//! ## Pipeline
//!
//! 1. `tauri_runtime_cef::audio::register_audio_handler` taps the
//!    per-browser `cef_audio_handler_t`. CEF delivers planar
//!    float32 PCM at the renderer's native rate (typically 48 kHz,
//!    1–2 channels) directly from the audio output device path —
//!    *before* it hits the OS speaker. No system permission needed.
//!
//! 2. Downsample-to-mono runs inline on the CEF audio thread:
//!    - average across channels → mono float32
//!    - linear-interpolate down to 16 kHz (the rate `voice::streaming`
//!      and the smoke test in `meet_agent::session` expect)
//!    - clamp + scale to PCM16LE
//!
//! 3. Accumulate ~100 ms per chunk (1 600 samples @ 16 kHz). We push
//!    via the core RPC on every flush boundary; smaller pushes would
//!    overload the JSON envelope, larger ones would slow VAD.
//!
//! 4. RPC pushes are spawned on the tokio runtime so the audio
//!    callback never blocks on network IO. A bounded channel
//!    backpressures: if core is wedged, we drop the oldest queued
//!    chunk rather than holding CEF's audio thread.

use std::sync::{Arc, Mutex};

use tauri_runtime_cef::audio::{
    register_audio_handler, AudioHandlerRegistration, AudioStreamEvent,
};
use tokio::sync::mpsc;

const TARGET_SAMPLE_RATE: u32 = 16_000;
/// 100 ms @ 16 kHz mono. `meet_agent::ops::Vad` pushes hangover counts
/// based on per-frame cadence, so changing this changes the VAD wall
/// time too. 100 ms feels responsive without burning RPC.
const FLUSH_SAMPLES: usize = (TARGET_SAMPLE_RATE as usize) / 10;
/// Bounded channel between the CEF callback (producer) and the
/// async-runtime forwarder (consumer). 32 chunks ≈ 3.2 s at the flush
/// cadence — generous slack for transient core latency, but bounded
/// so a wedged core can't OOM us.
const FORWARD_CHANNEL_CAPACITY: usize = 32;

/// RAII handle. Drop to release the CEF audio registration and shut
/// down the forwarder task. Both happen synchronously — the channel
/// closes first, the task exits its recv loop, and the registration
/// drop unhooks CEF in the same tick.
pub struct ListenSession {
    pub request_id: String,
    _registration: AudioHandlerRegistration,
    /// Held so `Drop` closes the channel even if there are no in-flight
    /// chunks. The forwarder task observes the close and exits.
    _shutdown_tx: mpsc::Sender<Vec<u8>>,
}

/// Opens the audio capture for `meet_url`. The same exact URL must
/// have been used to build the CEF window — `register_audio_handler`
/// matches by prefix.
pub fn start(meet_url: &str, request_id: String) -> Result<ListenSession, String> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>(FORWARD_CHANNEL_CAPACITY);
    let resampler = Arc::new(Mutex::new(Resampler::new()));

    let resampler_for_handler = resampler.clone();
    let tx_for_handler = tx.clone();
    let request_id_for_log = request_id.clone();
    let registration = register_audio_handler(meet_url.to_string(), move |event| {
        on_audio_event(
            &request_id_for_log,
            event,
            &resampler_for_handler,
            &tx_for_handler,
        );
    });

    spawn_forwarder(request_id.clone(), rx);

    log::info!(
        "[meet-audio] listen registered request_id={} url_chars={}",
        request_id,
        meet_url.chars().count()
    );

    Ok(ListenSession {
        request_id,
        _registration: registration,
        _shutdown_tx: tx,
    })
}

/// Process one CEF audio event. Speech/Stopped/Error all flow through
/// here; only `Packet` produces RPC traffic, but the others are logged
/// at info so an aborted call leaves a breadcrumb in the file logs.
fn on_audio_event(
    request_id: &str,
    event: AudioStreamEvent,
    resampler: &Arc<Mutex<Resampler>>,
    tx: &mpsc::Sender<Vec<u8>>,
) {
    match event {
        AudioStreamEvent::Started {
            sample_rate_hz,
            channels,
            frames_per_buffer,
        } => {
            log::info!(
                "[meet-audio] cef stream start request_id={request_id} hz={sample_rate_hz} channels={channels} frames_per_buffer={frames_per_buffer}"
            );
            if let Ok(mut r) = resampler.lock() {
                r.reset(sample_rate_hz as u32);
            }
        }
        AudioStreamEvent::Packet {
            channels: planes,
            pts_ms: _,
        } => {
            let pcm_bytes = match resampler.lock() {
                Ok(mut r) => r.feed_and_drain(&planes),
                Err(_) => return,
            };
            for chunk in pcm_bytes.chunks(FLUSH_SAMPLES * 2) {
                // `try_send` drops the chunk on a full channel rather
                // than blocking the CEF audio thread. Better to lose
                // a frame than to stall the renderer.
                if tx.try_send(chunk.to_vec()).is_err() {
                    log::warn!(
                        "[meet-audio] forward channel full; dropping {} bytes request_id={request_id}",
                        chunk.len()
                    );
                }
            }
        }
        AudioStreamEvent::Stopped => {
            log::info!("[meet-audio] cef stream stopped request_id={request_id}");
            if let Ok(mut r) = resampler.lock() {
                r.reset(0);
            }
        }
        AudioStreamEvent::Error(msg) => {
            log::warn!("[meet-audio] cef stream error request_id={request_id} msg={msg}");
        }
    }
}

/// Pull chunks off the bounded channel and POST each to core. Lives in
/// its own task so the CEF callback never blocks on HTTP.
fn spawn_forwarder(request_id: String, mut rx: mpsc::Receiver<Vec<u8>>) {
    tauri::async_runtime::spawn(async move {
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        while let Some(chunk) = rx.recv().await {
            let pcm_b64 = B64.encode(&chunk);
            let res = super::rpc_call(
                "openhuman.meet_agent_push_listen_pcm",
                serde_json::json!({
                    "request_id": request_id,
                    "pcm_base64": pcm_b64,
                }),
            )
            .await;
            if let Err(err) = res {
                log::debug!(
                    "[meet-audio] push_listen_pcm err request_id={request_id} bytes={} err={err}",
                    chunk.len()
                );
            }
        }
        log::info!("[meet-audio] forwarder exiting request_id={request_id}");
    });
}

/// Stateful float32-planar → PCM16LE mono @ 16 kHz resampler.
///
/// Uses linear interpolation, which is good enough for speech (the
/// downstream STT does not care about ultrasonics or pristine high
/// frequencies). Carry the previous sample across `feed_and_drain`
/// calls so we don't introduce a tick at every CEF buffer boundary.
/// Pick a source sample by signed index. Negative indices return the
/// carry sample from the previous call (so phase < 0 keeps the
/// interpolation continuous across buffer boundaries); past-the-end
/// indices clamp to the last sample (which is what the next call will
/// install as its own carry, so the output stays smooth even if a
/// caller stops feeding mid-stream).
fn sample_at(mono: &[f32], carry: f32, idx: i64) -> f32 {
    if idx < 0 {
        carry
    } else if (idx as usize) < mono.len() {
        mono[idx as usize]
    } else {
        *mono.last().unwrap_or(&0.0)
    }
}

struct Resampler {
    source_rate_hz: u32,
    /// Fractional position into the source buffer between calls.
    /// 0.0 means "start cleanly with the next sample". Negative is
    /// not used — the source rate is always known before we feed.
    phase: f64,
    /// Last source sample of the previous call, used as the "left"
    /// neighbour when we interpolate the first sample of the next call.
    last_sample: f32,
}

impl Resampler {
    fn new() -> Self {
        Self {
            source_rate_hz: 0,
            phase: 0.0,
            last_sample: 0.0,
        }
    }

    fn reset(&mut self, source_rate_hz: u32) {
        self.source_rate_hz = source_rate_hz;
        self.phase = 0.0;
        self.last_sample = 0.0;
    }

    fn feed_and_drain(&mut self, planes: &[Vec<f32>]) -> Vec<u8> {
        if planes.is_empty() || self.source_rate_hz == 0 {
            return Vec::new();
        }
        let frames = planes[0].len();
        if frames == 0 {
            return Vec::new();
        }
        // Mono mix.
        let mono: Vec<f32> = (0..frames)
            .map(|i| {
                let mut sum = 0.0_f32;
                for plane in planes {
                    if let Some(v) = plane.get(i) {
                        sum += *v;
                    }
                }
                sum / planes.len() as f32
            })
            .collect();

        let ratio = self.source_rate_hz as f64 / TARGET_SAMPLE_RATE as f64;
        let mut out = Vec::with_capacity((mono.len() as f64 / ratio).ceil() as usize * 2);
        // `pos` floats through `mono` indices. `pos < 0` means "still
        // sampling the carry sample from the previous call"; `pos = 0`
        // means "right at mono[0]".
        let mut pos = self.phase;
        while pos < mono.len() as f64 {
            let idx_f = pos.floor();
            let frac = pos - idx_f;
            let idx = idx_f as i64;
            let s_left = sample_at(mono.as_slice(), self.last_sample, idx);
            let s_right = sample_at(mono.as_slice(), self.last_sample, idx + 1);
            let sample = s_left as f64 + (s_right as f64 - s_left as f64) * frac;
            // Float32 [-1.0, 1.0] → i16. Clamp because Chromium can
            // overshoot a touch on heavy compression.
            let s_i16 = (sample.clamp(-1.0, 1.0) * i16::MAX as f64) as i16;
            out.extend_from_slice(&s_i16.to_le_bytes());
            pos += ratio;
        }
        // Carry the trailing fractional position into the next call.
        // It will be negative when we overshot (next call resumes
        // mid-source-sample), so the next call interpolates between
        // `last_sample` and the new mono[0].
        self.phase = pos - mono.len() as f64;
        self.last_sample = *mono.last().unwrap_or(&0.0);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_with_no_source_rate_yields_nothing() {
        let mut r = Resampler::new();
        let out = r.feed_and_drain(&[vec![0.5; 100]]);
        assert!(out.is_empty(), "no source rate set, must produce nothing");
    }

    #[test]
    fn resampler_48k_to_16k_mono_drops_samples_3to1() {
        let mut r = Resampler::new();
        r.reset(48_000);
        let plane = vec![0.5_f32; 4_800]; // 100ms @ 48k
        let bytes = r.feed_and_drain(&[plane]);
        // 100ms @ 16k = 1600 samples * 2 bytes. Allow ±2 samples slop
        // from the fractional phase carry.
        let samples = bytes.len() / 2;
        assert!(
            (1598..=1602).contains(&samples),
            "expected ~1600 samples, got {samples}"
        );
    }

    #[test]
    fn resampler_stereo_to_mono_averages_channels() {
        let mut r = Resampler::new();
        r.reset(16_000);
        let left = vec![0.8_f32; 1600];
        let right = vec![-0.2_f32; 1600];
        let bytes = r.feed_and_drain(&[left, right]);
        // Avg = 0.3 → ~9830 in i16. First two bytes are LE i16.
        let first = i16::from_le_bytes([bytes[0], bytes[1]]);
        assert!(
            (9000..11000).contains(&first),
            "expected mid-amplitude i16, got {first}"
        );
    }

    #[test]
    fn resampler_clamps_out_of_range_floats() {
        let mut r = Resampler::new();
        r.reset(16_000);
        let bytes = r.feed_and_drain(&[vec![5.0_f32; 100]]);
        let first = i16::from_le_bytes([bytes[0], bytes[1]]);
        assert_eq!(first, i16::MAX);
    }

    #[test]
    fn resampler_passthrough_when_rates_match() {
        let mut r = Resampler::new();
        r.reset(16_000);
        let plane = vec![0.5_f32; 1600];
        let bytes = r.feed_and_drain(&[plane]);
        assert_eq!(bytes.len(), 1600 * 2);
    }
}
