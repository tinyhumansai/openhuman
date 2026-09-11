use super::*;

#[test]
fn empty_input_is_none() {
    let r = scan("");
    assert_eq!(r.level, RiskLevel::None);
    assert_eq!(r.score, 0);
    assert!(r.categories.is_empty());
    assert!(!r.is_sensitive());
}

#[test]
fn level_thresholds_are_monotonic() {
    assert_eq!(level_from_score(0), RiskLevel::None);
    assert_eq!(level_from_score(1), RiskLevel::Low);
    assert_eq!(level_from_score(19), RiskLevel::Low);
    assert_eq!(level_from_score(20), RiskLevel::Medium);
    assert_eq!(level_from_score(44), RiskLevel::Medium);
    assert_eq!(level_from_score(45), RiskLevel::High);
    assert_eq!(level_from_score(1000), RiskLevel::High);
}
