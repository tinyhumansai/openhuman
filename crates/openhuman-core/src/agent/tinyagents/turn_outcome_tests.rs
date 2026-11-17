use super::*;

#[test]
fn an_all_zero_unobserved_turn_is_not_recorded() {
    assert!(!record_unobserved_turn_usage(
        &crate::core::runtime::DispatchCtx::ambient(),
        "m",
        0,
        0,
        0,
        None
    ));
}

#[test]
fn an_unobserved_turn_with_tokens_is_recorded_once() {
    assert!(record_unobserved_turn_usage(
        &crate::core::runtime::DispatchCtx::ambient(),
        "m",
        10,
        5,
        2,
        Some(0.01)
    ));
}
