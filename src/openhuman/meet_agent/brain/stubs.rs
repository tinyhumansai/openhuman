//! Stub fallbacks for STT/LLM/TTS used in tests and no-backend runs.

use super::constants::SAMPLE_RATE_HZ;

pub(super) async fn stub_stt(samples: &[i16]) -> String {
    let secs = samples.len() as f32 / SAMPLE_RATE_HZ as f32;
    format!("(heard ~{secs:.1}s of audio)")
}

#[allow(dead_code)]
pub(super) async fn stub_llm(_heard: &str) -> String {
    "I'm listening.".to_string()
}

pub(super) async fn stub_tts(text: &str) -> Vec<i16> {
    if text.is_empty() {
        return Vec::new();
    }
    let sample_rate = SAMPLE_RATE_HZ as f32;
    let freq = 440.0_f32;
    let duration_secs = 0.2_f32;
    let count = (sample_rate * duration_secs) as usize;
    (0..count)
        .map(|i| {
            let t = i as f32 / sample_rate;
            (((2.0 * std::f32::consts::PI * freq * t).sin()) * (i16::MAX as f32 * 0.3)) as i16
        })
        .collect()
}
