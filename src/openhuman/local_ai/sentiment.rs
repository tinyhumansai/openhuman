//! Emotion / sentiment analysis via the bundled local AI model.

use crate::openhuman::config::Config;
use crate::openhuman::local_ai;
use crate::rpc::RpcOutcome;

/// Result of sentiment / emotion analysis on a user message.
#[derive(Debug, serde::Serialize)]
pub struct SentimentResult {
    /// Primary emotion label.
    /// One of: joy, sadness, anger, surprise, fear, disgust, neutral.
    pub emotion: String,
    /// Overall valence: positive, negative, or neutral.
    pub valence: String,
    /// Model's self-reported confidence (0.0–1.0).
    pub confidence: f32,
}

impl SentimentResult {
    /// Safe default when analysis is skipped or parsing fails.
    fn neutral() -> Self {
        Self {
            emotion: "neutral".to_string(),
            valence: "neutral".to_string(),
            confidence: 1.0,
        }
    }
}

/// Known emotion labels the model is expected to produce.
const VALID_EMOTIONS: &[&str] = &[
    "joy", "sadness", "anger", "surprise", "fear", "disgust", "neutral",
];

/// Known valence labels.
const VALID_VALENCES: &[&str] = &["positive", "negative", "neutral"];

/// Ask the local model to classify the emotion and sentiment of a user
/// message. Designed to be called periodically (e.g. every hour), not on
/// every single message. Lightweight: ~8 output tokens, fire-and-forget safe.
pub async fn local_ai_analyze_sentiment(
    config: &Config,
    message: &str,
) -> Result<RpcOutcome<SentimentResult>, String> {
    tracing::debug!(
        msg_len = message.len(),
        "[local_ai:sentiment] evaluating sentiment"
    );

    if message.trim().is_empty() {
        return Ok(RpcOutcome::single_log(
            SentimentResult::neutral(),
            "empty message — neutral sentiment",
        ));
    }

    let service = local_ai::global(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        tracing::debug!("[local_ai:sentiment] local model not ready, returning neutral");
        return Ok(RpcOutcome::single_log(
            SentimentResult::neutral(),
            "local model not ready",
        ));
    }

    let prompt = format!(
        "Classify the emotion and sentiment of this user message.\n\
         Reply with EXACTLY three words separated by spaces:\n\
         EMOTION VALENCE CONFIDENCE\n\
         Where EMOTION is one of: joy, sadness, anger, surprise, fear, disgust, neutral\n\
         VALENCE is one of: positive, negative, neutral\n\
         CONFIDENCE is a number from 0.0 to 1.0\n\n\
         User message: {message}"
    );

    let output = service.prompt(config, &prompt, Some(8), true).await;

    let result = match output {
        Ok(raw) => {
            let trimmed = raw.trim().to_lowercase();
            tracing::debug!(
                raw = %trimmed,
                "[local_ai:sentiment] model response"
            );
            parse_sentiment_response(&trimmed)
        }
        Err(e) => {
            tracing::debug!(error = %e, "[local_ai:sentiment] inference failed, returning neutral");
            SentimentResult::neutral()
        }
    };

    tracing::debug!(
        emotion = %result.emotion,
        valence = %result.valence,
        confidence = result.confidence,
        "[local_ai:sentiment] analysis complete"
    );
    Ok(RpcOutcome::single_log(
        result,
        "sentiment analysis completed",
    ))
}

/// Parse the model's 3-word response into a `SentimentResult`.
/// Falls back to neutral on any parsing error.
fn parse_sentiment_response(text: &str) -> SentimentResult {
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() < 3 {
        tracing::debug!(
            parts = parts.len(),
            "[local_ai:sentiment] unexpected token count, falling back to neutral"
        );
        return SentimentResult::neutral();
    }

    let emotion = parts[0].to_string();
    let valence = parts[1].to_string();
    let confidence: f32 = parts[2].parse().unwrap_or(0.5);

    // Validate labels, fall back to neutral for garbage
    let emotion = if VALID_EMOTIONS.contains(&emotion.as_str()) {
        emotion
    } else {
        tracing::debug!(raw = %emotion, "[local_ai:sentiment] unknown emotion label, defaulting to neutral");
        "neutral".to_string()
    };

    let valence = if VALID_VALENCES.contains(&valence.as_str()) {
        valence
    } else {
        tracing::debug!(raw = %valence, "[local_ai:sentiment] unknown valence label, defaulting to neutral");
        "neutral".to_string()
    };

    let confidence = confidence.clamp(0.0, 1.0);

    SentimentResult {
        emotion,
        valence,
        confidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_response() {
        let r = parse_sentiment_response("joy positive 0.9");
        assert_eq!(r.emotion, "joy");
        assert_eq!(r.valence, "positive");
        assert!((r.confidence - 0.9).abs() < 0.01);
    }

    #[test]
    fn parse_valid_negative() {
        let r = parse_sentiment_response("anger negative 0.75");
        assert_eq!(r.emotion, "anger");
        assert_eq!(r.valence, "negative");
        assert!((r.confidence - 0.75).abs() < 0.01);
    }

    #[test]
    fn parse_unknown_emotion_falls_back() {
        let r = parse_sentiment_response("excited positive 0.8");
        assert_eq!(r.emotion, "neutral");
        assert_eq!(r.valence, "positive");
    }

    #[test]
    fn parse_too_few_tokens() {
        let r = parse_sentiment_response("joy");
        assert_eq!(r.emotion, "neutral");
        assert_eq!(r.valence, "neutral");
    }

    #[test]
    fn parse_bad_confidence() {
        let r = parse_sentiment_response("sadness negative abc");
        assert_eq!(r.emotion, "sadness");
        assert_eq!(r.valence, "negative");
        assert!((r.confidence - 0.5).abs() < 0.01);
    }

    #[test]
    fn parse_clamps_confidence() {
        let r = parse_sentiment_response("joy positive 2.5");
        assert!((r.confidence - 1.0).abs() < 0.01);
    }

    #[test]
    fn parse_empty_returns_neutral() {
        let r = parse_sentiment_response("");
        assert_eq!(r.emotion, "neutral");
        assert_eq!(r.valence, "neutral");
    }

    #[test]
    fn parse_clamps_negative_confidence_to_zero() {
        let r = parse_sentiment_response("joy positive -0.5");
        assert!(r.confidence >= 0.0 && r.confidence <= 1.0);
        assert!((r.confidence - 0.0).abs() < 0.01);
    }

    #[test]
    fn parse_unknown_valence_falls_back_to_neutral() {
        let r = parse_sentiment_response("joy mixed 0.8");
        assert_eq!(r.emotion, "joy");
        assert_eq!(r.valence, "neutral");
    }

    #[test]
    fn parse_accepts_all_documented_emotions() {
        for e in [
            "joy", "sadness", "anger", "surprise", "fear", "disgust", "neutral",
        ] {
            let r = parse_sentiment_response(&format!("{e} positive 0.5"));
            assert_eq!(r.emotion, e, "emotion `{e}` should be accepted verbatim");
        }
    }

    #[test]
    fn parse_accepts_all_documented_valences() {
        for v in ["positive", "negative", "neutral"] {
            let r = parse_sentiment_response(&format!("joy {v} 0.5"));
            assert_eq!(r.valence, v, "valence `{v}` should be accepted verbatim");
        }
    }

    #[test]
    fn neutral_constructor_returns_documented_defaults() {
        let r = SentimentResult::neutral();
        assert_eq!(r.emotion, "neutral");
        assert_eq!(r.valence, "neutral");
        assert!((r.confidence - 1.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn local_ai_analyze_sentiment_returns_neutral_for_empty_message() {
        let config = Config::default();
        let outcome = local_ai_analyze_sentiment(&config, "   ").await.unwrap();
        assert_eq!(outcome.value.emotion, "neutral");
        assert_eq!(outcome.value.valence, "neutral");
        assert!(outcome.logs.iter().any(|l| l.contains("empty message")));
    }
}
