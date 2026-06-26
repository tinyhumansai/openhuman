//! Wake word detection for the voice assistant.
//!
//! Uses a two-stage approach:
//! 1. Energy gate — only process audio chunks above a speech threshold
//! 2. STT keyword match — transcribe short chunks and check for the wake phrase
//!
//! This avoids running full STT on every audio frame while still providing
//! reliable keyword detection without a dedicated wake-word model.
//!
//! ## Log prefix
//!
//! `[voice-assistant-wake]`

use tracing::debug;

const LOG_PREFIX: &str = "[voice-assistant-wake]";

/// Minimum energy threshold to consider a chunk as potential speech.
/// ~-40dBFS for 16-bit audio (RMS ~100 → energy ~10000).
const ENERGY_GATE: f64 = 8_000.0;

/// Maximum chunk duration for wake word detection (1.5 seconds @ 16kHz).
const WAKE_CHUNK_SAMPLES: usize = 16_000 * 3 / 2;

/// Result of wake word detection on an audio chunk.
#[derive(Debug, Clone, PartialEq)]
pub enum WakeWordResult {
    /// No speech detected (below energy gate).
    Silence,
    /// Speech detected but wake word not found.
    SpeechNoMatch,
    /// Wake word detected — transcript contains the phrase.
    Detected { transcript: String },
}

/// Check if an audio chunk contains the wake word.
///
/// Stage 1: energy gate (fast, no STT needed for silence).
/// Stage 2: if energy is above threshold, returns `SpeechNoMatch` —
/// the caller should run STT and call `check_transcript` to verify.
pub fn check_audio_energy(samples: &[i16]) -> WakeWordResult {
    if samples.is_empty() {
        return WakeWordResult::Silence;
    }

    let energy: f64 = samples
        .iter()
        .map(|&s| (s as f64) * (s as f64))
        .sum::<f64>()
        / samples.len() as f64;

    if energy < ENERGY_GATE {
        WakeWordResult::Silence
    } else {
        debug!("{LOG_PREFIX} energy={energy:.0} above gate, speech candidate");
        WakeWordResult::SpeechNoMatch
    }
}

/// Check if a transcript contains the wake word phrase.
///
/// Uses fuzzy matching: the wake phrase must appear as a substring
/// (case-insensitive) in the transcript. Handles common STT variations
/// like "hey open human" vs "hey openhuman".
pub fn check_transcript(transcript: &str, wake_phrase: &str) -> WakeWordResult {
    let lower_transcript = transcript.to_lowercase();
    let lower_phrase = wake_phrase.to_lowercase();

    // Direct substring match.
    if lower_transcript.contains(&lower_phrase) {
        debug!("{LOG_PREFIX} wake word detected: \"{wake_phrase}\" in \"{transcript}\"");
        return WakeWordResult::Detected {
            transcript: transcript.to_string(),
        };
    }

    // Try without spaces (STT may merge words: "openhuman" vs "open human").
    let no_space_transcript: String = lower_transcript.chars().filter(|c| *c != ' ').collect();
    let no_space_phrase: String = lower_phrase.chars().filter(|c| *c != ' ').collect();
    if no_space_transcript.contains(&no_space_phrase) {
        debug!("{LOG_PREFIX} wake word detected (no-space match): \"{wake_phrase}\"");
        return WakeWordResult::Detected {
            transcript: transcript.to_string(),
        };
    }

    // Levenshtein-like: check if any window of phrase-length words is close enough.
    let phrase_words: Vec<&str> = lower_phrase.split_whitespace().collect();
    let transcript_words: Vec<&str> = lower_transcript.split_whitespace().collect();
    if phrase_words.len() <= transcript_words.len() {
        for window in transcript_words.windows(phrase_words.len()) {
            let matches = window
                .iter()
                .zip(phrase_words.iter())
                .filter(|(a, b)| words_similar(a, b))
                .count();
            // Allow 1 word mismatch for phrases > 2 words.
            let threshold = if phrase_words.len() > 2 {
                phrase_words.len() - 1
            } else {
                phrase_words.len()
            };
            if matches >= threshold {
                debug!("{LOG_PREFIX} wake word detected (fuzzy): \"{wake_phrase}\"");
                return WakeWordResult::Detected {
                    transcript: transcript.to_string(),
                };
            }
        }
    }

    WakeWordResult::SpeechNoMatch
}

/// Check if two words are similar enough (edit distance ≤ 1 for short words, ≤ 2 for longer).
fn words_similar(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let max_dist = if a.len().max(b.len()) <= 4 { 1 } else { 2 };
    strsim::levenshtein(a, b) <= max_dist
}

/// Get the recommended chunk size for wake word detection.
pub fn wake_chunk_size() -> usize {
    WAKE_CHUNK_SAMPLES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_below_gate() {
        let silence = vec![0i16; 1600];
        assert_eq!(check_audio_energy(&silence), WakeWordResult::Silence);
    }

    #[test]
    fn speech_above_gate() {
        let loud = vec![500i16; 1600];
        assert_eq!(check_audio_energy(&loud), WakeWordResult::SpeechNoMatch);
    }

    #[test]
    fn empty_is_silence() {
        assert_eq!(check_audio_energy(&[]), WakeWordResult::Silence);
    }

    #[test]
    fn exact_match() {
        let result = check_transcript("hey open human how are you", "hey open human");
        assert!(matches!(result, WakeWordResult::Detected { .. }));
    }

    #[test]
    fn case_insensitive_match() {
        let result = check_transcript("Hey Open Human", "hey open human");
        assert!(matches!(result, WakeWordResult::Detected { .. }));
    }

    #[test]
    fn no_space_match() {
        let result = check_transcript("heyopenhuman start", "hey open human");
        assert!(matches!(result, WakeWordResult::Detected { .. }));
    }

    #[test]
    fn fuzzy_match_one_word_off() {
        // "hey open hooman" — one word slightly different
        let result = check_transcript("hey open hooman", "hey open human");
        assert!(matches!(result, WakeWordResult::Detected { .. }));
    }

    #[test]
    fn no_match() {
        let result = check_transcript("what is the weather today", "hey open human");
        assert_eq!(result, WakeWordResult::SpeechNoMatch);
    }

    #[test]
    fn edit_distance_basic() {
        assert_eq!(strsim::levenshtein("kitten", "sitting"), 3);
        assert_eq!(strsim::levenshtein("hello", "hello"), 0);
        assert_eq!(strsim::levenshtein("human", "hooman"), 2);
    }
}
