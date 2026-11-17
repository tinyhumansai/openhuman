use super::runtime_session::OpenHumanSessionState;
use std::sync::Arc;

#[test]
fn a_response_shape_applies_to_exactly_the_next_turn() {
    use tinyagents_harness::middleware::{ResponseShape, ResponseShapeScope};
    let mut state = OpenHumanSessionState::default();
    let scope = ResponseShapeScope::new(ResponseShape::default());
    state.pending_response_shape = Some(Arc::clone(&scope));

    let first = state.take_turn_inputs();
    assert!(first
        .response_shape
        .as_ref()
        .is_some_and(|taken| Arc::ptr_eq(taken, &scope)));
    assert!(state.take_turn_inputs().response_shape.is_none());
}
