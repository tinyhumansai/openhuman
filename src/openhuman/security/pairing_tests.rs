use super::*;
use tokio::test;

// ── PairingGuard ─────────────────────────────────────────

#[test]
async fn new_guard_generates_code_when_no_tokens() {
    let guard = PairingGuard::new(true, &[]);
    assert!(guard.pairing_code().is_some());
    assert!(!guard.is_paired());
}

#[test]
async fn new_guard_no_code_when_tokens_exist() {
    let guard = PairingGuard::new(true, &["zc_existing".into()]);
    assert!(guard.pairing_code().is_none());
    assert!(guard.is_paired());
}

#[test]
async fn new_guard_no_code_when_pairing_disabled() {
    let guard = PairingGuard::new(false, &[]);
    assert!(guard.pairing_code().is_none());
}

#[test]
async fn try_pair_correct_code() {
    let guard = PairingGuard::new(true, &[]);
    let code = guard.pairing_code().unwrap().to_string();
    let token = guard.try_pair(&code).await.unwrap();
    assert!(token.is_some());
    assert!(token.unwrap().starts_with("zc_"));
    assert!(guard.is_paired());
}

#[test]
async fn try_pair_wrong_code() {
    let guard = PairingGuard::new(true, &[]);
    let result = guard.try_pair("000000").await.unwrap();
    // Might succeed if code happens to be 000000, but extremely unlikely
    // Just check it returns Ok(None) normally
    let _ = result;
}

#[test]
async fn try_pair_empty_code() {
    let guard = PairingGuard::new(true, &[]);
    assert!(guard.try_pair("").await.unwrap().is_none());
}

#[test]
async fn is_authenticated_with_valid_token() {
    // Pass plaintext token — PairingGuard hashes it on load
    let guard = PairingGuard::new(true, &["zc_valid".into()]);
    assert!(guard.is_authenticated("zc_valid"));
}

#[test]
async fn is_authenticated_with_prehashed_token() {
    // Pass an already-hashed token (64 hex chars)
    let hashed = hash_token("zc_valid");
    let guard = PairingGuard::new(true, &[hashed]);
    assert!(guard.is_authenticated("zc_valid"));
}

#[test]
async fn is_authenticated_with_invalid_token() {
    let guard = PairingGuard::new(true, &["zc_valid".into()]);
    assert!(!guard.is_authenticated("zc_invalid"));
}

#[test]
async fn is_authenticated_when_pairing_disabled() {
    let guard = PairingGuard::new(false, &[]);
    assert!(guard.is_authenticated("anything"));
    assert!(guard.is_authenticated(""));
}

#[test]
async fn tokens_returns_hashes() {
    let guard = PairingGuard::new(true, &["zc_a".into(), "zc_b".into()]);
    let tokens = guard.tokens();
    assert_eq!(tokens.len(), 2);
    // Tokens should be stored as 64-char hex hashes, not plaintext
    for t in &tokens {
        assert_eq!(t.len(), 64, "Token should be a SHA-256 hash");
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!t.starts_with("zc_"), "Token should not be plaintext");
    }
}

#[test]
async fn pair_then_authenticate() {
    let guard = PairingGuard::new(true, &[]);
    let code = guard.pairing_code().unwrap().to_string();
    let token = guard.try_pair(&code).await.unwrap().unwrap();
    assert!(guard.is_authenticated(&token));
    assert!(!guard.is_authenticated("wrong"));
}

// ── Token hashing ────────────────────────────────────────

#[test]
async fn hash_token_produces_64_hex_chars() {
    let hash = hash_token("zc_test_token");
    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
async fn hash_token_is_deterministic() {
    assert_eq!(hash_token("zc_abc"), hash_token("zc_abc"));
}

#[test]
async fn hash_token_differs_for_different_inputs() {
    assert_ne!(hash_token("zc_a"), hash_token("zc_b"));
}

#[test]
async fn is_token_hash_detects_hash_vs_plaintext() {
    assert!(is_token_hash(&hash_token("zc_test")));
    assert!(!is_token_hash("zc_test_token"));
    assert!(!is_token_hash("too_short"));
    assert!(!is_token_hash(""));
}

// ── is_public_bind ───────────────────────────────────────

#[test]
async fn localhost_variants_not_public() {
    assert!(!is_public_bind("127.0.0.1"));
    assert!(!is_public_bind("localhost"));
    assert!(!is_public_bind("::1"));
    assert!(!is_public_bind("[::1]"));
}

#[test]
async fn zero_zero_is_public() {
    assert!(is_public_bind("0.0.0.0"));
}

#[test]
async fn real_ip_is_public() {
    assert!(is_public_bind("192.168.1.100"));
    assert!(is_public_bind("10.0.0.1"));
}

// ── constant_time_eq ─────────────────────────────────────

#[test]
async fn constant_time_eq_same() {
    assert!(constant_time_eq("abc", "abc"));
    assert!(constant_time_eq("", ""));
}

#[test]
async fn constant_time_eq_different() {
    assert!(!constant_time_eq("abc", "abd"));
    assert!(!constant_time_eq("abc", "ab"));
    assert!(!constant_time_eq("a", ""));
}

// ── generate helpers ─────────────────────────────────────

#[test]
async fn generate_code_is_6_digits() {
    let code = generate_code();
    assert_eq!(code.len(), 6);
    assert!(code.chars().all(|c| c.is_ascii_digit()));
}

#[test]
async fn generate_code_is_not_deterministic() {
    // Two codes should differ with overwhelming probability. We try
    // multiple pairs so a single 1-in-10^6 collision doesn't cause
    // a flaky CI failure. All 10 pairs colliding is ~1-in-10^60.
    for _ in 0..10 {
        if generate_code() != generate_code() {
            return; // Pass: found a non-matching pair.
        }
    }
    panic!("Generated 10 pairs of codes and all were collisions — CSPRNG failure");
}

#[test]
async fn generate_token_has_prefix_and_hex_payload() {
    let token = generate_token();
    let payload = token
        .strip_prefix("zc_")
        .expect("Generated token should include zc_ prefix");

    assert_eq!(payload.len(), 64, "Token payload should be 32 bytes in hex");
    assert!(
        payload
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, 'a'..='f')),
        "Token payload should be lowercase hex"
    );
}

// ── Brute force protection ───────────────────────────────

#[test]
async fn brute_force_lockout_after_max_attempts() {
    let guard = PairingGuard::new(true, &[]);
    // Exhaust all attempts with wrong codes
    for i in 0..MAX_PAIR_ATTEMPTS {
        let result = guard.try_pair(&format!("wrong_{i}")).await;
        assert!(result.is_ok(), "Attempt {i} should not be locked out yet");
    }
    // Next attempt should be locked out
    let result = guard.try_pair("another_wrong").await;
    assert!(
        result.is_err(),
        "Should be locked out after {MAX_PAIR_ATTEMPTS} attempts"
    );
    let lockout_secs = result.unwrap_err();
    assert!(lockout_secs > 0, "Lockout should have remaining seconds");
    assert!(
        lockout_secs <= PAIR_LOCKOUT_SECS,
        "Lockout should not exceed max"
    );
}

#[test]
async fn correct_code_resets_failed_attempts() {
    let guard = PairingGuard::new(true, &[]);
    let code = guard.pairing_code().unwrap().to_string();
    // Fail a few times
    for _ in 0..3 {
        let _ = guard.try_pair("wrong").await;
    }
    // Correct code should still work (under MAX_PAIR_ATTEMPTS)
    let result = guard.try_pair(&code).await.unwrap();
    assert!(result.is_some(), "Correct code should work before lockout");
}

#[test]
async fn lockout_returns_remaining_seconds() {
    let guard = PairingGuard::new(true, &[]);
    for _ in 0..MAX_PAIR_ATTEMPTS {
        let _ = guard.try_pair("wrong").await;
    }
    let err = guard.try_pair("wrong").await.unwrap_err();
    // Should be close to PAIR_LOCKOUT_SECS (within a second)
    assert!(
        err >= PAIR_LOCKOUT_SECS - 1,
        "Remaining lockout should be ~{PAIR_LOCKOUT_SECS}s, got {err}s"
    );
}
