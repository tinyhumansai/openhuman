use super::*;

fn record(owner: &str, epoch: u64, expires_at_ms: u64, released: bool) -> LeaseRecord {
    LeaseRecord {
        owner: owner.to_string(),
        endpoint: None,
        epoch,
        expires_at_ms,
        released,
    }
}

#[test]
fn keys_cannot_escape_a_directory_or_an_id() {
    for good in [
        "user-1",
        "a",
        "alice@example.com",
        "x_y.z",
        &"k".repeat(MAX_KEY_LEN),
    ] {
        assert!(validate_key(good).is_ok(), "{good}");
    }
    let too_long = "k".repeat(MAX_KEY_LEN + 1);
    for bad in [
        "", ".", "..", ".hidden", "a/b", "a\\b", "a b", "a\0b", "a:b", "é", &too_long,
    ] {
        assert!(validate_key(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn liveness_and_retry_after_follow_expiry_and_release() {
    let live = record("n1", 1, 100, false);
    assert!(live.is_live(99));
    assert!(!live.is_live(100), "expiry is exclusive");
    assert_eq!(live.retry_after_ms(40), 60);
    assert_eq!(live.retry_after_ms(400), 0);
    let released = record("n1", 1, 100, true);
    assert!(!released.is_live(0));
    assert_eq!(released.retry_after_ms(0), 0);
}

#[test]
fn a_fresh_key_starts_clean_at_epoch_one() {
    assert_eq!(
        decide(None, "n1", None, 0),
        Takeover::Take {
            epoch: 1,
            unclean: false
        }
    );
}

#[test]
fn a_released_record_is_taken_cleanly_at_the_next_epoch() {
    let found = record("n2", 4, u64::MAX, true);
    assert_eq!(
        decide(Some(&found), "n1", None, 0),
        Takeover::Take {
            epoch: 5,
            unclean: false
        }
    );
}

#[test]
fn a_live_foreign_record_is_refused_until_it_expires() {
    let found = record("n2", 4, 100, false);
    assert_eq!(decide(Some(&found), "n1", None, 99), Takeover::Refuse);
    assert_eq!(
        decide(Some(&found), "n1", None, 100),
        Takeover::Take {
            epoch: 5,
            unclean: true
        }
    );
}

#[test]
fn own_records_are_reentrant_only_for_the_holding_instance() {
    let found = record("n1", 4, 100, false);
    assert_eq!(
        decide(Some(&found), "n1", Some(4), 50),
        Takeover::Take {
            epoch: 4,
            unclean: false
        },
        "the same instance re-acquires without a new epoch"
    );
    assert_eq!(
        decide(Some(&found), "n1", None, 50),
        Takeover::Take {
            epoch: 5,
            unclean: true
        },
        "a restarted node finds its dead incarnation's record"
    );
    assert_eq!(
        decide(Some(&found), "n1", Some(3), 50),
        Takeover::Take {
            epoch: 5,
            unclean: true
        },
        "a stale held epoch is not re-entrant"
    );
}

#[test]
fn lease_errors_name_the_holder() {
    let error = LeaseError::Held(record("n2", 3, 10, false));
    assert_eq!(error.to_string(), "lease held by n2 (epoch 3)");
    assert_eq!(LeaseError::Lost.to_string(), "lease lost");
}

#[test]
fn expired_reentrant_record_advances_the_epoch() {
    let found = record("n1", 4, 100, false);
    assert_eq!(
        decide(Some(&found), "n1", Some(4), 99),
        Takeover::Take {
            epoch: 4,
            unclean: false
        }
    );
    assert_eq!(
        decide(Some(&found), "n1", Some(4), 100),
        Takeover::Take {
            epoch: 5,
            unclean: true
        }
    );
}

#[test]
fn exhausted_epoch_is_refused_not_reused() {
    let found = record("n2", u64::MAX, 100, false);
    assert_eq!(decide(Some(&found), "n1", None, 100), Takeover::Exhausted);
}

#[test]
fn local_dirs_do_not_alias_by_case() {
    let root = std::path::Path::new("/r");
    assert_ne!(local_dir(root, "alice"), local_dir(root, "ALICE"));
    assert_eq!(local_dir(root, "alice"), local_dir(root, "alice"));
}
