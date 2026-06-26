//! Voice assistant brain — STT → LLM → TTS orchestration.
//!
//! Runs a single conversational turn: drains inbound PCM from the session,
//! transcribes via the configured STT provider, sends the transcript to the
//! LLM, synthesizes the reply via the configured TTS provider, and enqueues
//! the resulting PCM on the session's outbound buffer.
//!
//! ## Log prefix
//!
//! `[voice-assistant-brain]` — grep-friendly for end-to-end traces.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use tracing::{debug, info, warn};

use crate::openhuman::config::Config;
use crate::openhuman::meet_agent::wav::{pack_pcm16le_mono_wav, strip_for_speech};
use crate::openhuman::voice::factory::{create_stt_provider, create_tts_provider};

use super::session::SessionRegistry;
use super::types::SessionState;

const LOG_PREFIX: &str = "[voice-assistant-brain]";

// Per-session noise cancel state (evicted on session stop).
static NC_STATES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, super::noise_cancel::NoiseCancelState>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Remove noise cancel state for a stopped session (prevents memory leak).
pub fn evict_nc_state(session_id: &str) {
    if let Ok(mut states) = NC_STATES.lock() {
        states.remove(session_id);
    }
}

/// Run a single voice assistant turn for the given session.
///
/// Called when VAD detects end-of-utterance. The session must exist and
/// have inbound PCM buffered.
pub async fn run_turn(session_id: &str) -> Result<(), String> {
    // Guard: verify session still exists before proceeding (prevents race
    // where session is stopped between lock acquisition and task execution).
    SessionRegistry::with_session(session_id, |_| ())?;

    info!("{LOG_PREFIX} turn started session={session_id}");

    // 1. Mark session as processing and drain inbound PCM.
    let (pcm, stt_provider_name, tts_provider_name, language, history) =
        SessionRegistry::with_session(session_id, |s| {
            s.state = SessionState::Processing;
            let pcm = s.drain_inbound_pcm();
            let history: Vec<(String, String)> = s
                .history
                .iter()
                .map(|t| (t.user_text.clone(), t.assistant_text.clone()))
                .collect();
            (
                pcm,
                s.stt_provider.clone(),
                s.tts_provider.clone(),
                s.language.clone(),
                history,
            )
        })?;

    if pcm.is_empty() {
        debug!("{LOG_PREFIX} no PCM buffered, skipping turn session={session_id}");
        SessionRegistry::with_session(session_id, |s| {
            s.state = SessionState::Listening;
        })?;
        return Ok(());
    }

    // 1b. Apply noise cancellation before STT (per-session adaptive state).
    let pcm = {
        use super::noise_cancel::{NoiseCancelConfig, NoiseCancelState};

        let mut states = NC_STATES.lock().unwrap_or_else(|e| e.into_inner());
        let nc = states
            .entry(session_id.to_string())
            .or_insert_with(|| NoiseCancelState::new(NoiseCancelConfig::default()));
        nc.process(&pcm, None)
    };

    debug!(
        "{LOG_PREFIX} draining {} samples ({:.2}s) session={session_id}",
        pcm.len(),
        pcm.len() as f64 / 16_000.0
    );

    // 2. STT: PCM → text. Use streaming for longer audio (>4s).
    let stt_start = std::time::Instant::now();
    let config = crate::openhuman::config::ops::load_config_with_timeout()
        .await
        .map_err(|e| format!("{LOG_PREFIX} config load failed: {e}"))?;
    let transcript = if pcm.len() > 16_000 * 4 {
        run_streaming_stt(
            session_id,
            &pcm,
            &config,
            &stt_provider_name,
            language.as_deref(),
        )
        .await?
    } else {
        run_stt(&config, &pcm, &stt_provider_name, language.as_deref()).await?
    };

    if transcript.trim().is_empty() {
        debug!("{LOG_PREFIX} empty transcript, skipping LLM session={session_id}");
        SessionRegistry::with_session(session_id, |s| {
            s.state = SessionState::Listening;
        })?;
        return Ok(());
    }

    let stt_ms = stt_start.elapsed().as_millis();
    info!(
        "{LOG_PREFIX} STT result: \"{}\" ({stt_ms}ms) session={session_id}",
        truncate(&transcript, 80)
    );

    // 2b. Detect emotion/sentiment from transcript (non-blocking, best-effort).
    let emotion = detect_emotion(&transcript);
    // 2c. Detect language from transcript (heuristic, updates session).
    let detected_lang = detect_language(&transcript);
    // 2d. Auto-switch language if detected language differs from session language.
    SessionRegistry::with_session(session_id, |s| {
        s.detected_emotion = emotion;
        if let Some(ref lang) = detected_lang {
            // Auto-switch: update session language for next STT pass.
            if s.language.as_deref() != Some(lang) {
                debug!(
                    "{LOG_PREFIX} auto-switching language: {:?} -> {lang} session={session_id}",
                    s.language
                );
                s.language = Some(lang.clone());
            }
        }
        s.detected_language = detected_lang;
    })?;

    // 3. LLM: transcript + history → reply.
    let llm_start = std::time::Instant::now();
    let reply = run_llm(&config, &transcript, &history).await?;
    let llm_ms = llm_start.elapsed().as_millis();

    info!(
        "{LOG_PREFIX} LLM reply: \"{}\" ({llm_ms}ms) session={session_id}",
        truncate(&reply, 80)
    );

    // 4. Streaming TTS: split reply into sentence chunks, synthesize and enqueue
    //    progressively so playback starts before full synthesis completes.
    let tts_start = std::time::Instant::now();
    let sentences = split_into_sentences(&reply);
    let chunk_count = sentences.len();

    SessionRegistry::with_session(session_id, |s| {
        s.state = SessionState::Speaking;
    })?;

    for (i, sentence) in sentences.iter().enumerate() {
        if sentence.trim().is_empty() {
            continue;
        }
        let tts_pcm = run_tts(&config, sentence, &tts_provider_name).await?;
        debug!(
            "{LOG_PREFIX} TTS chunk {}/{} produced {} samples ({:.2}s) session={session_id}",
            i + 1,
            chunk_count,
            tts_pcm.len(),
            tts_pcm.len() as f64 / 16_000.0
        );
        // Check for barge-in between chunks.
        let interrupted = SessionRegistry::with_session(session_id, |s| {
            if s.state != SessionState::Speaking {
                true
            } else {
                s.enqueue_outbound_pcm(&tts_pcm);
                false
            }
        })?;
        if interrupted {
            info!("{LOG_PREFIX} barge-in during streaming TTS, stopping at chunk {}/{} session={session_id}", i + 1, chunk_count);
            break;
        }
    }

    // 5. Record turn and transition back.
    let tts_ms = tts_start.elapsed().as_millis();
    SessionRegistry::with_session(session_id, |s| {
        s.record_turn(&transcript, &reply);
        if s.state == SessionState::Speaking {
            s.state = SessionState::Listening;
        }
    })?;

    info!("{LOG_PREFIX} turn completed session={session_id} latency: stt={stt_ms}ms llm={llm_ms}ms tts={tts_ms}ms total={}ms", stt_ms + llm_ms + tts_ms);
    Ok(())
}

// ---------------------------------------------------------------------------
// STT
// ---------------------------------------------------------------------------

async fn run_stt(
    config: &Config,
    pcm: &[i16],
    provider_name: &str,
    language: Option<&str>,
) -> Result<String, String> {
    let provider = create_stt_provider(provider_name, "", config)
        .map_err(|e| format!("{LOG_PREFIX} STT provider creation failed: {e}"))?;

    // Pack PCM into WAV and base64-encode for the provider interface.
    let wav_bytes = pack_pcm16le_mono_wav(pcm, 16_000);
    let audio_b64 = B64.encode(&wav_bytes);

    debug!(
        "{LOG_PREFIX} STT dispatch provider={} wav_bytes={} b64_len={}",
        provider.name(),
        wav_bytes.len(),
        audio_b64.len()
    );

    let outcome = provider
        .transcribe(config, &audio_b64, Some("audio/wav"), None, language)
        .await
        .map_err(|e| format!("{LOG_PREFIX} STT failed: {e}"))?;

    Ok(outcome.value.text)
}

// ---------------------------------------------------------------------------
// LLM
// ---------------------------------------------------------------------------

async fn run_llm(
    config: &Config,
    transcript: &str,
    history: &[(String, String)],
) -> Result<String, String> {
    use crate::openhuman::inference::provider::create_chat_provider;
    use crate::openhuman::inference::provider::traits::ChatMessage;

    let (provider, model) = create_chat_provider("agentic", config)
        .map_err(|e| format!("{LOG_PREFIX} LLM provider creation failed: {e}"))?;

    // Build messages with conversation history.
    let mut messages = vec![ChatMessage::system(
        "You are a helpful voice assistant. Keep responses concise and conversational — \
         the user is speaking to you and will hear your reply read aloud. \
         Avoid markdown, code blocks, or long lists unless explicitly asked.",
    )];

    // Add conversation history (last 10 turns max for context window).
    for (user, assistant) in history.iter().rev().take(10).rev() {
        messages.push(ChatMessage::user(user));
        messages.push(ChatMessage::assistant(assistant));
    }

    messages.push(ChatMessage::user(transcript));

    debug!(
        "{LOG_PREFIX} LLM request messages={} transcript_len={}",
        messages.len(),
        transcript.len()
    );

    let text = provider
        .chat_with_history(&messages, &model, 0.5)
        .await
        .map_err(|e| format!("{LOG_PREFIX} LLM request failed: {e}"))?;

    Ok(strip_for_speech(&text))
}

// ---------------------------------------------------------------------------
// TTS
// ---------------------------------------------------------------------------

async fn run_tts(config: &Config, text: &str, provider_name: &str) -> Result<Vec<i16>, String> {
    let provider = create_tts_provider(provider_name, "", config)
        .map_err(|e| format!("{LOG_PREFIX} TTS provider creation failed: {e}"))?;

    debug!(
        "{LOG_PREFIX} TTS dispatch provider={} text_len={}",
        provider.name(),
        text.len()
    );

    let outcome = provider
        .synthesize(config, text, None)
        .await
        .map_err(|e| format!("{LOG_PREFIX} TTS failed: {e}"))?;

    let result = outcome.value;

    // Decode the base64 audio into PCM16LE samples.
    let audio_bytes = B64
        .decode(&result.audio_base64)
        .map_err(|e| format!("{LOG_PREFIX} TTS audio decode failed: {e}"))?;

    // The audio may be WAV-wrapped or raw PCM depending on provider.
    // Try to strip WAV header if present (44 bytes for standard RIFF/WAVE).
    let pcm_bytes = if audio_bytes.len() > 44 && &audio_bytes[0..4] == b"RIFF" {
        &audio_bytes[44..]
    } else {
        &audio_bytes
    };

    if pcm_bytes.len() % 2 != 0 {
        warn!(
            "{LOG_PREFIX} TTS returned odd byte count {}, truncating last byte",
            pcm_bytes.len()
        );
    }

    let samples: Vec<i16> = pcm_bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();

    Ok(samples)
}

// ---------------------------------------------------------------------------
// Emotion / Sentiment Detection
// ---------------------------------------------------------------------------

/// Detect emotion from transcript text using keyword heuristics.
/// Returns None if neutral/uncertain. LLM-based detection is a future enhancement.
fn detect_emotion(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let positive = [
        "happy",
        "great",
        "awesome",
        "love",
        "excited",
        "wonderful",
        "fantastic",
        "thank",
    ];
    let negative = [
        "angry",
        "frustrated",
        "annoyed",
        "hate",
        "terrible",
        "awful",
        "upset",
        "furious",
    ];
    let urgent = [
        "help",
        "emergency",
        "urgent",
        "asap",
        "immediately",
        "critical",
    ];
    let confused = [
        "confused",
        "don't understand",
        "what do you mean",
        "unclear",
        "lost",
    ];

    for w in &urgent {
        if lower.contains(w) {
            return Some("urgent".into());
        }
    }
    for w in &negative {
        if lower.contains(w) {
            return Some("negative".into());
        }
    }
    for w in &confused {
        if lower.contains(w) {
            return Some("confused".into());
        }
    }
    for w in &positive {
        if lower.contains(w) {
            return Some("positive".into());
        }
    }
    None
}

/// Detect language from transcript using trigram analysis (whatlang crate).
/// Returns BCP-47 code or None if English (default).
fn detect_language(text: &str) -> Option<String> {
    let info = whatlang::detect(text)?;
    if info.confidence() < 0.5 {
        return None;
    }
    let code = match info.lang() {
        whatlang::Lang::Eng => return None, // English is default
        whatlang::Lang::Cmn => "zh",
        whatlang::Lang::Spa => "es",
        whatlang::Lang::Fra => "fr",
        whatlang::Lang::Deu => "de",
        whatlang::Lang::Rus => "ru",
        whatlang::Lang::Ara => "ar",
        whatlang::Lang::Hin => "hi",
        whatlang::Lang::Jpn => "ja",
        whatlang::Lang::Kor => "ko",
        whatlang::Lang::Por => "pt",
        whatlang::Lang::Ita => "it",
        whatlang::Lang::Nld => "nl",
        whatlang::Lang::Tur => "tr",
        whatlang::Lang::Pol => "pl",
        whatlang::Lang::Ukr => "uk",
        whatlang::Lang::Tha => "th",
        whatlang::Lang::Vie => "vi",
        whatlang::Lang::Ind => "id",
        whatlang::Lang::Swe => "sv",
        other => {
            debug!(
                "{LOG_PREFIX} detected lang {:?} conf={:.2}",
                other,
                info.confidence()
            );
            return Some(format!("{:?}", other).to_lowercase());
        }
    };
    Some(code.into())
}

// ---------------------------------------------------------------------------
// Streaming STT (chunked whisper with partial results)
// ---------------------------------------------------------------------------

/// Minimum chunk size for streaming STT (2 seconds @ 16kHz).
const STREAMING_CHUNK_SIZE: usize = 16_000 * 2;

/// Process audio in chunks and emit partial transcripts.
/// Uses LocalAgreement-2 approach: only emit text that appears in 2 consecutive runs.
pub async fn run_streaming_stt(
    session_id: &str,
    pcm: &[i16],
    config: &Config,
    provider_name: &str,
    language: Option<&str>,
) -> Result<String, String> {
    if pcm.len() < STREAMING_CHUNK_SIZE {
        // Too short for streaming — just do a single pass.
        return run_stt(config, pcm, provider_name, language).await;
    }

    let mut confirmed = String::new();
    let mut prev_output = String::new();
    let chunk_size = STREAMING_CHUNK_SIZE;
    let mut offset = 0;

    while offset < pcm.len() {
        let end = (offset + chunk_size * 2).min(pcm.len()); // Process 2x chunk for overlap
        let chunk = &pcm[..end];

        let current_output = run_stt(config, chunk, provider_name, language).await?;

        // LocalAgreement: find longest common prefix between prev and current.
        if !prev_output.is_empty() {
            let agreement = longest_common_prefix(&prev_output, &current_output);
            if agreement.len() > confirmed.len() {
                // Update partial transcript on session.
                let partial = agreement.clone();
                let _ = SessionRegistry::with_session(session_id, |s| {
                    s.partial_transcript = partial;
                });
                confirmed = agreement;
            }
        }

        prev_output = current_output;
        offset += chunk_size;
    }

    // Final output is the last full transcription.
    let final_text = if prev_output.len() > confirmed.len() {
        prev_output
    } else {
        confirmed
    };

    // Clear partial transcript.
    let _ = SessionRegistry::with_session(session_id, |s| {
        s.partial_transcript.clear();
    });

    Ok(final_text)
}

/// Find the longest common prefix of two strings (word-aligned).
fn longest_common_prefix(a: &str, b: &str) -> String {
    let a_words: Vec<&str> = a.split_whitespace().collect();
    let b_words: Vec<&str> = b.split_whitespace().collect();
    let common_count = a_words
        .iter()
        .zip(b_words.iter())
        .take_while(|(x, y)| x == y)
        .count();
    a_words[..common_count].join(" ")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // Find the last char boundary at or before `max` to avoid panicking on multi-byte UTF-8.
        let end = s.floor_char_boundary(max);
        &s[..end]
    }
}

/// Split text into sentence-level chunks for streaming TTS.
/// Uses UAX#29 sentence boundaries (handles abbreviations, decimals, non-Latin).
fn split_into_sentences(text: &str) -> Vec<String> {
    use unicode_segmentation::UnicodeSegmentation;
    let sentences: Vec<String> = text
        .unicode_sentences()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if sentences.is_empty() {
        vec![text.to_string()]
    } else {
        sentences
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string() {
        let long = "a".repeat(100);
        assert_eq!(truncate(&long, 10).len(), 10);
    }

    #[test]
    fn truncate_multibyte_utf8_no_panic() {
        // Each emoji is 4 bytes. Slicing at byte 5 would split a char and panic without floor_char_boundary.
        let s = "😀😀😀";
        let result = truncate(s, 5);
        assert_eq!(result, "😀"); // 4 bytes fits, 8 doesn't
    }

    #[test]
    fn strip_for_speech_removes_markdown() {
        assert_eq!(strip_for_speech("**bold** text"), "bold text");
        assert_eq!(
            strip_for_speech("- item one\n- item two"),
            "item one item two"
        );
        assert_eq!(strip_for_speech("```\ncode\n```\nafter"), "after");
    }
}
