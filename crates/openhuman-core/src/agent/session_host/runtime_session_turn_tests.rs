use super::*;

#[test]
fn runtime_turn_error_restores_the_typed_refusal_before_string_fallback() {
    let context = OpenHumanRunContext::new();
    let refusal = crate::agent::tinyagents::budget::DispatchRefusal::new(
        crate::agent::tinyagents::budget::DispatchRefusalReason::CallLimit,
    );
    context.session_sidecar.lock().unwrap().dispatch_refusal = Some(refusal);

    let error = runtime_turn_error(
        tinyagents_runtime::RuntimeError::Driver("driver failed".to_owned()),
        &context.session_sidecar,
    );

    assert_eq!(
        error
            .downcast_ref::<crate::agent::tinyagents::budget::DispatchRefusal>()
            .expect("the typed refusal crosses the runtime boundary")
            .reason(),
        refusal.reason()
    );
    assert!(context
        .session_sidecar
        .lock()
        .unwrap()
        .dispatch_refusal
        .is_none());
}

#[test]
fn runtime_turn_error_uses_generic_message_when_no_refusal_was_recorded() {
    let context = OpenHumanRunContext::new();
    let error = runtime_turn_error(
        tinyagents_runtime::RuntimeError::Driver("unrelated driver failure".to_owned()),
        &context.session_sidecar,
    );

    assert!(error
        .downcast_ref::<crate::agent::tinyagents::budget::DispatchRefusal>()
        .is_none());
    assert!(error.to_string().contains("unrelated driver failure"));
}

#[test]
fn a_later_runtime_cancellation_is_not_reclassified_from_a_stale_sidecar() {
    let context = OpenHumanRunContext::new();
    context.session_sidecar.lock().unwrap().dispatch_refusal =
        Some(crate::agent::tinyagents::budget::DispatchRefusal::new(
            crate::agent::tinyagents::budget::DispatchRefusalReason::CallLimit,
        ));

    let error = runtime_turn_error(
        tinyagents_runtime::RuntimeError::Cancelled,
        &context.session_sidecar,
    );

    assert!(error
        .downcast_ref::<crate::agent::tinyagents::budget::DispatchRefusal>()
        .is_none());
    assert_eq!(error.to_string(), "session turn cancelled");
}

#[test]
fn a_resumed_session_with_no_rows_starts_its_thread() {
    // The store created the session but has written nothing to it.
    assert!(starts_thread(false, &[]));
    assert!(starts_thread(true, &[]));
}

#[test]
fn a_session_with_imported_rows_does_not_start_a_thread() {
    use tinyagents_session::transcript::TranscriptMessage;
    let imported = [TranscriptMessage::user("legacy question")];
    assert!(!starts_thread(false, &imported));
}
