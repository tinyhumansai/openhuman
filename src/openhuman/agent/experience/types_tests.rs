use super::*;

#[test]
fn redact_text_masks_secret_like_values() {
    let redacted = redact_text("token=abc123 password: hunter2 normal");
    assert!(redacted.contains("token=[redacted]"));
    assert!(redacted.contains("password: [redacted]"));
    assert!(!redacted.contains("abc123"));
    assert!(!redacted.contains("hunter2"));
    assert!(redacted.contains("normal"));
}

#[test]
fn redact_text_masks_bearer_tokens_and_openai_style_keys() {
    let redacted =
        redact_text("Authorization: Bearer secret-token sk-abcdefghijklmnopqrstuvwxyz123456");
    assert!(!redacted.contains("secret-token"));
    assert!(!redacted.contains("sk-abcdefghijklmnopqrstuvwxyz123456"));
    assert!(redacted.contains("Bearer [redacted]"));
    assert!(redacted.contains("sk-[redacted]"));
}

#[test]
fn stable_experience_id_is_repeatable() {
    let sequence = vec!["grep".to_string(), "file_read".to_string()];
    let first = stable_experience_id("same task", &sequence, ExperienceOutcome::Success);
    let second = stable_experience_id("same task", &sequence, ExperienceOutcome::Success);
    assert_eq!(first, second);
    assert!(first.starts_with("exp_"));
}

#[test]
fn stable_experience_id_changes_when_outcome_changes() {
    let sequence = vec!["grep".to_string(), "file_read".to_string()];
    let success = stable_experience_id("same task", &sequence, ExperienceOutcome::Success);
    let failure = stable_experience_id("same task", &sequence, ExperienceOutcome::Failure);
    assert_ne!(success, failure);
}

#[test]
fn stable_experience_id_for_profile_none_matches_legacy_derivation() {
    // `None` must be byte-identical to the pre-1c derivation so existing
    // stored records keep their identity.
    let sequence = vec!["grep".to_string(), "file_read".to_string()];
    let legacy = stable_experience_id("same task", &sequence, ExperienceOutcome::Success);
    let none =
        stable_experience_id_for_profile("same task", &sequence, ExperienceOutcome::Success, None);
    assert_eq!(legacy, none);
    // An empty / whitespace-only profile id is treated as `None`.
    let blank = stable_experience_id_for_profile(
        "same task",
        &sequence,
        ExperienceOutcome::Success,
        Some("   "),
    );
    assert_eq!(legacy, blank);
}

#[test]
fn stable_experience_id_for_profile_partitions_by_profile() {
    // Same task/tool/outcome triple under profile A vs B vs None yields three
    // distinct keys, so no profile can overwrite another's record.
    let sequence = vec!["grep".to_string(), "file_read".to_string()];
    let none =
        stable_experience_id_for_profile("same task", &sequence, ExperienceOutcome::Success, None);
    let alice = stable_experience_id_for_profile(
        "same task",
        &sequence,
        ExperienceOutcome::Success,
        Some("alice"),
    );
    let bob = stable_experience_id_for_profile(
        "same task",
        &sequence,
        ExperienceOutcome::Success,
        Some("bob"),
    );
    assert_ne!(none, alice);
    assert_ne!(none, bob);
    assert_ne!(alice, bob);
    // Deterministic per profile.
    let alice_again = stable_experience_id_for_profile(
        "same task",
        &sequence,
        ExperienceOutcome::Success,
        Some("alice"),
    );
    assert_eq!(alice, alice_again);
}
