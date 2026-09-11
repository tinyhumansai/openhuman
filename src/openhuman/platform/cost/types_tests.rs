use super::*;

#[test]
fn token_usage_calculation() {
    let usage = TokenUsage::new("test/model", 1000, 500, 3.0, 15.0);

    // Expected: (1000/1M)*3 + (500/1M)*15 = 0.003 + 0.0075 = 0.0105
    assert!((usage.cost_usd - 0.0105).abs() < 0.0001);
    assert_eq!(usage.input_tokens, 1000);
    assert_eq!(usage.output_tokens, 500);
    assert_eq!(usage.total_tokens, 1500);
}

#[test]
fn token_usage_zero_tokens() {
    let usage = TokenUsage::new("test/model", 0, 0, 3.0, 15.0);
    assert!(usage.cost_usd.abs() < f64::EPSILON);
    assert_eq!(usage.total_tokens, 0);
}

#[test]
fn token_usage_negative_or_non_finite_prices_are_clamped() {
    let usage = TokenUsage::new("test/model", 1000, 1000, -3.0, f64::NAN);
    assert!(usage.cost_usd.abs() < f64::EPSILON);
    assert_eq!(usage.total_tokens, 2000);
}

#[test]
fn cost_record_creation() {
    let usage = TokenUsage::new("test/model", 100, 50, 1.0, 2.0);
    let record = CostRecord::new("session-123", usage);

    assert_eq!(record.session_id, "session-123");
    assert!(!record.id.is_empty());
    assert_eq!(record.usage.model, "test/model");
}
