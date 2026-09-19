use super::*;

#[test]
fn resume_token_survives_old_cleanup_and_stop_cancels_the_successor() {
    let run_id = format!("workflow-cancel-race-{}", uuid::Uuid::new_v4());
    let old = register_cancel_signal(&run_id);
    let successor = replace_cancel_signal(&run_id);

    clear_cancel_signal(&run_id, &old);
    assert!(is_current_cancel_signal(&run_id, &successor));
    assert!(!successor.token.is_cancelled());

    assert!(cancel_signal_if_current(&run_id, &successor));
    assert!(successor.token.is_cancelled());
    assert!(successor.flag.load(Ordering::SeqCst));
    assert!(!old.token.is_cancelled());

    clear_cancel_signal(&run_id, &successor);
}
