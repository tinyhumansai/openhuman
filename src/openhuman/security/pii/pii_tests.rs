//! Fixture + boundary tests for the local PII detector.
//!
//! Coverage intent:
//! * every category detects its canonical form,
//! * false-positive guards (benign text, invalid card/IP) do NOT flag,
//! * false-negative / recall guards (varied formats, in-sentence values) DO
//!   flag — recall matters more than precision here,
//! * risk-level escalation behaves as specified,
//! * detection is deterministic + offline (no external egress on classify),
//! * category identifiers serialize to stable snake_case strings.

use super::rules::{is_ipv4, is_luhn_valid};
use super::{scan, PiiCategory, RiskLevel};

// ---------------------------------------------------------------------------
// Per-category detection (recall / true positives)
// ---------------------------------------------------------------------------

#[test]
fn detects_email() {
    let r = scan("reach me at alice.smith@example.com any time");
    assert!(r.has_category(PiiCategory::Email));
    assert!(r.is_sensitive());
}

#[test]
fn detects_national_id_ssn() {
    // Dash- and space-separated both count (recall).
    for raw in ["SSN 123-45-6789", "ssn is 123 45 6789"] {
        let r = scan(raw);
        assert!(r.has_category(PiiCategory::NationalId), "missed in {raw:?}");
        assert_eq!(r.level, RiskLevel::High, "strong id must be High: {raw:?}");
    }
}

#[test]
fn detects_valid_credit_card() {
    // 4111 1111 1111 1111 is the canonical Visa test number (Luhn-valid).
    for raw in [
        "card 4111 1111 1111 1111 exp 12/26",
        "4111-1111-1111-1111",
        "4111111111111111",
    ] {
        let r = scan(raw);
        assert!(r.has_category(PiiCategory::CreditCard), "missed in {raw:?}");
        assert_eq!(r.level, RiskLevel::High);
    }
}

#[test]
fn detects_iban_bank_account() {
    let r = scan("transfer to GB82WEST12345698765432 please");
    assert!(r.has_category(PiiCategory::BankAccount));
    assert_eq!(r.level, RiskLevel::High);
}

#[test]
fn detects_phone_numbers_varied_formats() {
    for raw in [
        "call 415-555-1234",
        "(415) 555 1234",
        "+1 415.555.1234",
        "reach +44 20 7946 0958 in london",
    ] {
        let r = scan(raw);
        assert!(
            r.has_category(PiiCategory::PhoneNumber),
            "missed in {raw:?}"
        );
    }
}

#[test]
fn detects_ipv4_address() {
    let r = scan("client connected from 192.168.10.24 last night");
    assert!(r.has_category(PiiCategory::IpAddress));
}

#[test]
fn detects_postal_address() {
    let r = scan("ship to 1600 Pennsylvania Avenue, Washington");
    assert!(r.has_category(PiiCategory::PostalAddress));
}

#[test]
fn detects_date_of_birth_context() {
    for raw in [
        "date of birth: 1990-04-12",
        "DOB 04/12/1990",
        "he was born on the 3rd",
    ] {
        let r = scan(raw);
        assert!(
            r.has_category(PiiCategory::DateOfBirth),
            "missed in {raw:?}"
        );
    }
}

#[test]
fn detects_passport() {
    let r = scan("upload a scan of your passport");
    assert!(r.has_category(PiiCategory::Passport));
    assert_eq!(r.level, RiskLevel::High); // strong identifier
}

#[test]
fn detects_medical_context() {
    let r = scan("the patient was diagnosed and prescribed medication");
    assert!(r.has_category(PiiCategory::Medical));
    assert!(r.level >= RiskLevel::Medium);
}

#[test]
fn detects_legal_context() {
    let r = scan("the plaintiff filed a lawsuit; the defendant's attorney responded");
    assert!(r.has_category(PiiCategory::Legal));
    assert!(r.level >= RiskLevel::Medium);
}

#[test]
fn detects_financial_context() {
    let r = scan("please send your bank account and routing number");
    assert!(r.has_category(PiiCategory::Financial));
    assert!(r.level >= RiskLevel::Medium);
}

#[test]
fn detects_family_context() {
    let r = scan("my spouse and our minor child are dependents");
    assert!(r.has_category(PiiCategory::Family));
    assert!(r.is_sensitive());
}

// ---------------------------------------------------------------------------
// False-positive guards
// ---------------------------------------------------------------------------

#[test]
fn benign_text_is_not_flagged() {
    for raw in [
        "let's grab coffee at 3pm tomorrow",
        "the build passed and the tests are green",
        "refactor the parser to be faster",
        "version 1.2.3 shipped yesterday", // dotted, but only 3 octets — not IPv4
    ] {
        let r = scan(raw);
        assert_eq!(r.level, RiskLevel::None, "false positive on {raw:?}: {r:?}");
        assert!(r.categories.is_empty(), "categories on {raw:?}: {r:?}");
    }
}

#[test]
fn luhn_invalid_number_is_not_a_card() {
    // 16 digits but fails the Luhn checksum → must NOT be a CreditCard.
    let r = scan("order id 1234 5678 9012 3456 confirmed");
    assert!(
        !r.has_category(PiiCategory::CreditCard),
        "Luhn-invalid number flagged as card: {r:?}"
    );
}

#[test]
fn out_of_range_octets_are_not_ipv4() {
    let r = scan("coords 300.400.500.600 are nonsense");
    assert!(
        !r.has_category(PiiCategory::IpAddress),
        "bad octets flagged: {r:?}"
    );
}

#[test]
fn phone_shaped_ssn_not_double_counted_as_ssn() {
    // 3-3-4 grouping is a phone, not a 3-2-4 SSN.
    let r = scan("call 415-555-1234");
    assert!(r.has_category(PiiCategory::PhoneNumber));
    assert!(!r.has_category(PiiCategory::NationalId));
}

// ---------------------------------------------------------------------------
// Risk-level escalation
// ---------------------------------------------------------------------------

#[test]
fn single_weak_identifier_is_low() {
    let r = scan("email me: bob@example.org");
    assert_eq!(r.level, RiskLevel::Low);
    assert_eq!(r.categories, vec![PiiCategory::Email]);
}

#[test]
fn co_occurring_identifiers_escalate() {
    // Email + phone together → beyond a single weak signal.
    let r = scan("bob@example.org / 415-555-1234");
    assert!(r.has_category(PiiCategory::Email));
    assert!(r.has_category(PiiCategory::PhoneNumber));
    assert!(
        r.level >= RiskLevel::Medium,
        "expected escalation, got {r:?}"
    );
}

#[test]
fn strong_identifier_forces_high_even_alone() {
    let r = scan("123-45-6789");
    assert_eq!(r.level, RiskLevel::High);
    assert!(r.score >= PiiCategory::NationalId.weight());
}

#[test]
fn medical_record_shape_is_high() {
    let r = scan("patient John Doe, DOB 01/02/1980, diagnosed; SSN 111-22-3333");
    assert_eq!(r.level, RiskLevel::High);
    assert!(r.has_category(PiiCategory::Medical));
    assert!(r.has_category(PiiCategory::NationalId));
    assert!(r.has_category(PiiCategory::DateOfBirth));
}

#[test]
fn score_increases_with_more_categories() {
    let one = scan("bob@example.org").score;
    let two = scan("bob@example.org and patient diagnosed").score;
    assert!(
        two > one,
        "adding a category should raise the score: {one} !< {two}"
    );
}

// ---------------------------------------------------------------------------
// Offline / determinism (no external egress on classify)
// ---------------------------------------------------------------------------

#[test]
fn scan_is_deterministic_and_pure() {
    // Detection performs zero I/O: it is a pure function of its input, so
    // repeated calls must be byte-for-byte identical. This is the unit-test
    // proxy for "no external egress occurs during classify" — there is no
    // async, no network client, and nothing in this module to mock away.
    let input = "patient bob@example.org, card 4111 1111 1111 1111, SSN 123-45-6789";
    let first = scan(input);
    let second = scan(input);
    assert_eq!(first, second);
}

#[test]
fn scan_runs_without_a_tokio_runtime() {
    // A plain `#[test]` has no Tokio runtime installed; the fact that `scan`
    // returns here at all proves it does not spawn async work / touch the
    // network to classify.
    let r = scan("SSN 123-45-6789");
    assert_eq!(r.level, RiskLevel::High);
}

// ---------------------------------------------------------------------------
// Stable serde identifiers (contract for S2 / RPC / event bus)
// ---------------------------------------------------------------------------

#[test]
fn category_identifiers_are_stable_snake_case() {
    let cases = [
        (PiiCategory::Email, "email"),
        (PiiCategory::PhoneNumber, "phone_number"),
        (PiiCategory::NationalId, "national_id"),
        (PiiCategory::CreditCard, "credit_card"),
        (PiiCategory::BankAccount, "bank_account"),
        (PiiCategory::IpAddress, "ip_address"),
        (PiiCategory::PostalAddress, "postal_address"),
        (PiiCategory::DateOfBirth, "date_of_birth"),
        (PiiCategory::Passport, "passport"),
        (PiiCategory::Medical, "medical"),
        (PiiCategory::Legal, "legal"),
        (PiiCategory::Financial, "financial"),
        (PiiCategory::Family, "family"),
    ];
    for (cat, expected) in cases {
        assert_eq!(cat.as_str(), expected);
        assert_eq!(
            serde_json::to_string(&cat).unwrap(),
            format!("\"{expected}\"")
        );
    }
}

#[test]
fn risk_levels_are_ordered_and_stable() {
    assert!(RiskLevel::None < RiskLevel::Low);
    assert!(RiskLevel::Low < RiskLevel::Medium);
    assert!(RiskLevel::Medium < RiskLevel::High);
    for (lvl, s) in [
        (RiskLevel::None, "none"),
        (RiskLevel::Low, "low"),
        (RiskLevel::Medium, "medium"),
        (RiskLevel::High, "high"),
    ] {
        assert_eq!(lvl.as_str(), s);
        assert_eq!(serde_json::to_string(&lvl).unwrap(), format!("\"{s}\""));
    }
}

// ---------------------------------------------------------------------------
// Validator unit tests
// ---------------------------------------------------------------------------

#[test]
fn luhn_validator_accepts_known_good_and_rejects_bad() {
    assert!(is_luhn_valid("4111111111111111")); // Visa test number
    assert!(is_luhn_valid("4111 1111 1111 1111"));
    assert!(!is_luhn_valid("1234567890123456"));
    assert!(!is_luhn_valid("12345")); // too short
}

#[test]
fn ipv4_validator_bounds_octets() {
    assert!(is_ipv4("192.168.0.1"));
    assert!(is_ipv4("255.255.255.255"));
    assert!(!is_ipv4("256.1.1.1"));
    assert!(!is_ipv4("1.2.3"));
}
