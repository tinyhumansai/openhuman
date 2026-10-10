use super::*;

#[test]
fn exited_members_do_not_block_cleanup_but_running_members_do() {
    for state in ["R", "S", "D", "T", "I"] {
        assert!(is_active_member(
            &format!("12 (name with ) spaces) {state} 1 42 42"),
            42
        ));
    }
    for state in ["Z", "X", "x"] {
        assert!(!is_active_member(
            &format!("12 (sleep) {state} 1 42 42"),
            42
        ));
    }
    assert!(!is_active_member("12 (sleep) R 1 43 43", 42));
    assert!(!is_active_member("malformed", 42));
}
