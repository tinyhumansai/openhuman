//! Detection rules for the local PII detector.
//!
//! Two flavours of rule, both compiled once into a process-wide static:
//!
//! * **Pattern rules** — a [`Regex`] plus an optional structural `validator`
//!   (Luhn for cards, octet range for IPv4) that rejects shape-matches that
//!   aren't actually valid identifiers.
//! * **Keyword rules** — an alternation of domain terms wrapped in word
//!   boundaries, used for the topical categories (medical / legal / financial /
//!   family) where there is no single canonical value shape.
//!
//! All matching is case-insensitive and offline; nothing here performs I/O.

use std::sync::LazyLock;

use regex::Regex;

use super::types::PiiCategory;

/// A single detection rule: matches [`regex`](Rule::regex), attributes hits to
/// [`category`](Rule::category), and — if present — only counts a match when
/// [`validator`](Rule::validator) confirms it.
pub(crate) struct Rule {
    pub category: PiiCategory,
    pub regex: Regex,
    /// Optional structural check applied to each raw match before it counts.
    pub validator: Option<fn(&str) -> bool>,
}

/// Build a keyword rule: a case-insensitive, word-boundaried alternation over
/// `terms`, all attributed to `category`.
fn keyword_rule(category: PiiCategory, terms: &[&str]) -> Rule {
    let alternation = terms.join("|");
    let pattern = format!(r"(?i)\b(?:{alternation})\b");
    Rule {
        category,
        regex: Regex::new(&pattern).expect("keyword rule regex is well-formed"),
        validator: None,
    }
}

/// Build a pattern rule from a raw regex string.
fn pattern_rule(category: PiiCategory, pattern: &str, validator: Option<fn(&str) -> bool>) -> Rule {
    Rule {
        category,
        regex: Regex::new(pattern).expect("pattern rule regex is well-formed"),
        validator,
    }
}

/// Luhn checksum validation for candidate payment-card numbers. Strips
/// separators first; requires 13–19 digits.
pub(crate) fn is_luhn_valid(raw: &str) -> bool {
    let digits: Vec<u8> = raw
        .bytes()
        .filter(|b| b.is_ascii_digit())
        .map(|b| b - b'0')
        .collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let mut sum = 0u32;
    // Double every second digit from the right.
    for (i, &d) in digits.iter().rev().enumerate() {
        let mut v = d as u32;
        if i % 2 == 1 {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
    }
    sum % 10 == 0
}

/// Validate that a dotted-quad match is a real IPv4 address (each octet 0–255).
pub(crate) fn is_ipv4(raw: &str) -> bool {
    let octets: Vec<&str> = raw.split('.').collect();
    octets.len() == 4
        && octets
            .iter()
            .all(|o| !o.is_empty() && o.parse::<u32>().is_ok_and(|n| n <= 255))
}

/// The full, process-wide rule set. Compiled lazily on first [`scan`] call and
/// reused for the life of the process.
///
/// [`scan`]: super::detector::scan
pub(crate) fn rules() -> &'static [Rule] {
    static RULES: LazyLock<Vec<Rule>> = LazyLock::new(build_rules);
    &RULES
}

fn build_rules() -> Vec<Rule> {
    vec![
        // --- structured identifiers -------------------------------------
        // Email.
        pattern_rule(
            PiiCategory::Email,
            r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}\b",
            None,
        ),
        // US Social Security Number (3-2-4, dash or space separated). The
        // separator requirement keeps it distinct from phone numbers (3-3-4)
        // and from bare 9-digit blobs.
        pattern_rule(
            PiiCategory::NationalId,
            r"\b\d{3}[-\s]\d{2}[-\s]\d{4}\b",
            None,
        ),
        // Payment card: 13–19 digits with optional single separators, then
        // Luhn-validated so version strings / IDs don't false-positive.
        pattern_rule(
            PiiCategory::CreditCard,
            r"\b\d(?:[ -]?\d){12,18}\b",
            Some(is_luhn_valid),
        ),
        // IBAN bank account: 2-letter country + 2 check digits + 11–30 alnum.
        pattern_rule(
            PiiCategory::BankAccount,
            r"\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b",
            None,
        ),
        // US-style phone number (3-3-4 groups, requires a separator so it
        // doesn't swallow arbitrary 10-digit numbers).
        pattern_rule(
            PiiCategory::PhoneNumber,
            r"\b(?:\+?1[-.\s]?)?\(?\d{3}\)?[-.\s]\d{3}[-.\s]\d{4}\b",
            None,
        ),
        // International E.164-ish phone number (leading + and 7–15 digits).
        pattern_rule(PiiCategory::PhoneNumber, r"\+\d(?:[\d\s\-]{6,16}\d)", None),
        // IPv4 address (octet-validated).
        pattern_rule(
            PiiCategory::IpAddress,
            r"\b(?:\d{1,3}\.){3}\d{1,3}\b",
            Some(is_ipv4),
        ),
        // Street / postal address (number + words + street-type keyword).
        pattern_rule(
            PiiCategory::PostalAddress,
            r"(?i)\b\d{1,6}\s+[A-Za-z0-9.'\- ]{2,40}?\b(?:street|st|avenue|ave|road|rd|boulevard|blvd|lane|ln|drive|dr|court|ct|way|place|pl|terrace|ter|circle|cir|highway|hwy)\b",
            None,
        ),
        // Date of birth (explicit context — pattern-based, not a keyword list).
        pattern_rule(
            PiiCategory::DateOfBirth,
            r"(?i)\b(?:date of birth|d\.?o\.?b\.?|birth ?date|born on)\b",
            None,
        ),
        // --- sensitive topical context (keyword-based) ------------------
        keyword_rule(PiiCategory::Passport, &["passport"]),
        keyword_rule(
            PiiCategory::Medical,
            &[
                "patient",
                "diagnosis",
                "diagnosed",
                "prescription",
                "prescribed",
                "symptom",
                "symptoms",
                "medication",
                "medications",
                "treatment",
                "physician",
                "hospital",
                "clinic",
                "disease",
                "medical record",
                "health record",
                "mrn",
                "blood pressure",
                "dosage",
                "therapy",
                "biopsy",
                "oncology",
                "cardiology",
                "psychiatric",
                "mental health",
                "allergy",
                "allergies",
            ],
        ),
        keyword_rule(
            PiiCategory::Legal,
            &[
                "plaintiff",
                "defendant",
                "attorney",
                "lawsuit",
                "litigation",
                "case number",
                "docket",
                "subpoena",
                "settlement",
                "indictment",
                "deposition",
                "legal counsel",
                "prosecution",
                "testimony",
                "affidavit",
                "arraignment",
                "felony",
                "misdemeanor",
                "custody",
                "court order",
            ],
        ),
        keyword_rule(
            PiiCategory::Financial,
            &[
                "bank account",
                "account number",
                "routing number",
                "salary",
                "annual income",
                "credit score",
                "tax id",
                "invoice",
                "sort code",
                "swift code",
                "wire transfer",
                "mortgage",
                "loan balance",
                "net worth",
                "iban",
            ],
        ),
        keyword_rule(
            PiiCategory::Family,
            &[
                "spouse",
                "wife",
                "husband",
                "daughter",
                "son",
                "children",
                "minor child",
                "maiden name",
                "next of kin",
                "dependent",
                "marital status",
                "guardian",
                "sibling",
            ],
        ),
    ]
}
