use super::*;
use crate::security::approval::APPROVAL_UNANSWERED_PHRASE;
use tinytools::status::POLICY_DENIED_MARKER;

#[test]
fn a_denied_secret_grant_says_the_person_denied_it() {
    let error = master_grant_refusal(&format!(
        "{POLICY_DENIED_MARKER} User denied 'native_secret_grant' execution. Do not re-request \
         the same call this turn; take a different approach or stop."
    ));
    assert_eq!(
        error.to_string(),
        "the approval to unlock configuration secrets was denied"
    );
}

#[test]
fn an_unanswered_secret_grant_says_it_timed_out() {
    let error = master_grant_refusal(&format!(
        "{POLICY_DENIED_MARKER} Approval for 'native_secret_grant' timed out after 30s: \
         {APPROVAL_UNANSWERED_PHRASE}, so it was not run."
    ));
    assert_eq!(
        error.to_string(),
        "the approval to unlock configuration secrets was not answered in time"
    );
}

#[test]
fn any_other_refusal_keeps_the_gates_reason_without_its_marker() {
    let error = master_grant_refusal(&format!(
        "{POLICY_DENIED_MARKER} 'native_secret_grant' requires an interactive WebChat approval."
    ));
    let shown = error.to_string();
    assert_eq!(
        shown,
        "the approval to unlock configuration secrets was refused: 'native_secret_grant' \
         requires an interactive WebChat approval."
    );
    assert!(!shown.contains(POLICY_DENIED_MARKER));
}
