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
