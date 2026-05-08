//! Turn orchestration: STT → LLM → TTS.
//!
//! ## Why a separate `brain` module
//!
//! `session.rs` owns the *state* (buffers, VAD, event log). The brain
//! is the *behavior* — when VAD reports end-of-utterance, take the
//! drained PCM, run STT, decide whether the agent should respond, run
//! TTS, and feed the synthesized PCM back into the session's outbound
//! queue. Splitting state-from-behavior is what makes the session
//! testable in isolation (the smoke tests in `session.rs` pin behavior
//! contracts without spinning up STT/TTS), and lets PR3 swap the
//! placeholder LLM call for the real `crate::openhuman::agent` runtime
//! without churning the session contract.
//!
//! ## Status
//!
//! PR1 ships a stub that records a "Heard" / "Spoke" event and enqueues
//! a tiny synthesized blip so the end-to-end audio path is exercised
//! without a real STT/TTS bill. PR3 wires the real adapters:
//!   - STT: `crate::openhuman::voice::cloud_transcribe`
//!     (which goes to ElevenLabs via the hosted backend)
//!   - LLM: `crate::openhuman::agent` runtime, channel="meet"
//!   - TTS: `crate::openhuman::voice::reply_speech`
//!     (also ElevenLabs via hosted backend; returns base64 audio +
//!     visemes which we throw away here — the mascot lipsync only
//!     fires on the OpenHuman main window, not the embedded Meet one).

use super::session::registry;
use super::types::SessionEventKind;

/// Fire a brain turn for the named session. Drains the inbound buffer,
/// runs STT/LLM/TTS, enqueues outbound. Returns `Ok(true)` when a turn
/// actually ran, `Ok(false)` when there was nothing to do (utterance
/// was empty / under the floor).
///
/// Best-effort: a failure inside STT/TTS is logged and surfaced as a
/// `Note` event, never as a panic that would tear down the session.
pub async fn run_turn(request_id: &str) -> Result<bool, String> {
    let drained = registry().with_session(request_id, |s| s.drain_inbound())?;
    if drained.len() < 4_000 {
        // Less than ~250ms at 16kHz — almost certainly a VAD false
        // positive (a cough, a click). Skip the round-trip.
        log::debug!(
            "[meet-agent] skipping turn request_id={request_id} samples={}",
            drained.len()
        );
        return Ok(false);
    }

    log::info!(
        "[meet-agent] turn start request_id={request_id} samples={}",
        drained.len()
    );

    // ─── STT ────────────────────────────────────────────────────────
    // PR1 stub: pretend the user said "(hello)". The real adapter call
    // is gated behind a feature follow-up; the placeholder still
    // exercises the event/transcript path and lets the smoke test
    // assert `turn_count` increments.
    let heard = stub_stt(&drained).await;
    log::info!(
        "[meet-agent] STT request_id={request_id} text_chars={}",
        heard.chars().count()
    );

    // ─── LLM (decide whether + what to reply) ───────────────────────
    let reply_text = stub_brain(&heard).await;

    // ─── TTS ────────────────────────────────────────────────────────
    let synthesized = stub_tts(&reply_text).await;

    registry().with_session(request_id, |s| {
        s.record_event(SessionEventKind::Heard, heard.clone());
        if !reply_text.is_empty() {
            s.record_event(SessionEventKind::Spoke, reply_text.clone());
            s.enqueue_outbound_pcm(&synthesized, true);
        } else {
            s.record_event(
                SessionEventKind::Note,
                "agent declined to respond".to_string(),
            );
        }
        s.turn_count += 1;
    })?;

    log::info!(
        "[meet-agent] turn done request_id={request_id} reply_chars={} synth_samples={}",
        reply_text.chars().count(),
        synthesized.len()
    );
    Ok(true)
}

/// Placeholder STT — returns a deterministic string proportional to the
/// input length so tests can pin behavior without network. Replaced in
/// PR3 with a real `voice::cloud_transcribe` call.
async fn stub_stt(samples: &[i16]) -> String {
    let secs = samples.len() as f32 / 16_000.0;
    format!("(heard ~{secs:.1}s of audio)")
}

/// Placeholder LLM — echoes a canned phrase so the speak path is
/// exercised without an LLM bill. Replaced in PR3 with a meet-channel
/// turn through `crate::openhuman::agent`.
async fn stub_brain(_heard: &str) -> String {
    "I'm listening.".to_string()
}

/// Placeholder TTS — synthesizes a 200ms 440Hz sine wave (PCM16 @ 16kHz)
/// proportional to the reply length so we have non-zero audio to push
/// out. Replaced in PR3 with `voice::reply_speech`.
async fn stub_tts(text: &str) -> Vec<i16> {
    if text.is_empty() {
        return Vec::new();
    }
    let sample_rate = 16_000.0_f32;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::meet_agent::session::registry;

    #[tokio::test]
    async fn run_turn_skips_short_buffers() {
        registry().start("brain-skip", 16_000).unwrap();
        registry()
            .with_session("brain-skip", |s| {
                s.push_inbound_pcm(&vec![0; 800]); // 50ms — under floor
            })
            .unwrap();
        assert_eq!(run_turn("brain-skip").await.unwrap(), false);
        let _ = registry().stop("brain-skip");
    }

    #[tokio::test]
    async fn run_turn_records_heard_and_spoke() {
        registry().start("brain-full", 16_000).unwrap();
        registry()
            .with_session("brain-full", |s| {
                s.push_inbound_pcm(&vec![1000; 16_000]); // 1s
            })
            .unwrap();
        assert_eq!(run_turn("brain-full").await.unwrap(), true);
        registry()
            .with_session("brain-full", |s| {
                let kinds: Vec<_> = s
                    .events()
                    .iter()
                    .map(|e| format!("{:?}", e.kind))
                    .collect();
                assert!(kinds.contains(&"Heard".to_string()));
                assert!(kinds.contains(&"Spoke".to_string()));
                assert_eq!(s.turn_count, 1);
                assert!(s.spoken_seconds() > 0.0);
            })
            .unwrap();
        let _ = registry().stop("brain-full");
    }
}
