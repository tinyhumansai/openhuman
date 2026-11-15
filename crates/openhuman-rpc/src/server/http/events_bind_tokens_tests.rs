use super::*;

#[test]
fn issued_token_validates_for_matching_client_id() {
    let issued = issue("cli-test-1", None).expect("issue");
    assert!(consume("cli-test-1", &issued.token));
}

#[test]
fn issued_token_rejects_wrong_client_id() {
    let issued = issue("cli-test-2", None).expect("issue");
    assert!(!consume("attacker-id", &issued.token));
}

#[test]
fn wrong_client_id_does_not_consume_token() {
    // Mismatched consume must leave the token intact so the legitimate
    // subscriber can still validate after the failed probe — otherwise
    // a wrong-id request becomes a one-shot DoS.
    let issued = issue("cli-test-mismatch", None).expect("issue");
    assert!(!consume("attacker-id", &issued.token));
    assert!(
        consume("cli-test-mismatch", &issued.token),
        "legitimate consume must still succeed after a mismatched probe"
    );
}

#[test]
fn consumed_token_cannot_be_reused() {
    let issued = issue("cli-test-3", None).expect("issue");
    assert!(consume("cli-test-3", &issued.token));
    assert!(
        !consume("cli-test-3", &issued.token),
        "tokens must be single-shot"
    );
}

#[test]
fn expired_token_is_rejected() {
    let issued = issue("cli-test-4", Some(Duration::from_millis(1))).expect("issue");
    std::thread::sleep(Duration::from_millis(20));
    assert!(!consume("cli-test-4", &issued.token));
}

#[test]
fn unknown_token_is_rejected() {
    assert!(!consume("any-id", "f00ba1"));
}

#[test]
fn ttl_override_is_clamped_to_max() {
    // Any caller asking for more than `MAX_TTL` collapses to the cap;
    // confirm the issue path does not panic and the resulting token
    // still validates.
    let issued = issue("cli-test-clamp", Some(Duration::from_secs(60 * 60 * 24))).expect("issue");
    assert!(issued.valid_until <= Instant::now() + MAX_TTL + Duration::from_secs(1));
    assert!(consume("cli-test-clamp", &issued.token));
}
