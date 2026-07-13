//! Result + category types for the local PII / identification-risk detector
//! (privacy epic #4256, slice S5).
//!
//! ## Privacy note
//! The detector deliberately reports **categories and counts only** — never
//! the raw matched substrings. The whole point of S5 is to reason about
//! identification risk *without* copying the sensitive value anywhere, so the
//! result type itself must not become a new PII sink (mirrors the "no response
//! body in telemetry" rule the codebase already follows).

use serde::{Deserialize, Serialize};

/// Identification-risk level assigned to a scanned piece of content.
///
/// Ordered `None < Low < Medium < High` (declaration order drives the derived
/// `Ord`), so callers can gate behaviour with a plain comparison such as
/// `result.level >= RiskLevel::Medium`.
///
/// The detector is tuned toward **recall**: a false positive (flagging benign
/// text) is far cheaper than a false negative (letting a real identifier leave
/// the machine unflagged), so the thresholds lean toward escalation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// No identification-risk signal found.
    #[default]
    None,
    /// A single weak signal (e.g. one email or phone number in isolation).
    Low,
    /// A moderate signal — sensitive topical context, or a couple of
    /// co-occurring identifiers.
    Medium,
    /// A strong signal — a high-confidence identifier (SSN, card, passport,
    /// bank account) or several co-occurring categories that together look
    /// like a real record.
    High,
}

impl RiskLevel {
    /// Stable lowercase identifier (matches the serde representation).
    pub fn as_str(self) -> &'static str {
        match self {
            RiskLevel::None => "none",
            RiskLevel::Low => "low",
            RiskLevel::Medium => "medium",
            RiskLevel::High => "high",
        }
    }

    /// `true` when the content carries any identification risk at all
    /// (anything above [`RiskLevel::None`]). Downstream gates (S3 indicator,
    /// S4 sensitive-transfer gate) use this as the coarse "should we care?"
    /// signal.
    pub fn is_sensitive(self) -> bool {
        self > RiskLevel::None
    }
}

/// A single identification-risk category the detector knows how to recognise.
///
/// The set covers both **structured identifiers** (pattern-matched: email,
/// phone, national ID/SSN, card, bank account, IP, postal address) and
/// **sensitive topical context** (keyword-matched: medical, legal, financial,
/// family, date-of-birth, passport) — the category set called out in the S5
/// scope (patient / legal / financial / family / personal identifiers).
///
/// Serde uses stable `snake_case` identifiers so the values are safe to carry
/// across the RPC / event-bus boundary and into the S2 egress descriptor
/// without a breaking reshape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiiCategory {
    // --- structured identifiers (pattern-based) ---
    /// Email address (`alice@example.com`).
    Email,
    /// Telephone number (US or international formats).
    PhoneNumber,
    /// National identifier — US Social Security Number and similar
    /// dash/space-separated government IDs.
    NationalId,
    /// Payment card number (validated with the Luhn checksum).
    CreditCard,
    /// Bank account number in IBAN form.
    BankAccount,
    /// IPv4 address (octet-validated).
    IpAddress,
    /// Street / postal address.
    PostalAddress,
    // --- sensitive topical context (keyword-based) ---
    /// Date of birth (explicit "DOB" / "date of birth" context).
    DateOfBirth,
    /// Passport reference.
    Passport,
    /// Medical / patient / health information.
    Medical,
    /// Legal / litigation / case information.
    Legal,
    /// Financial / banking / income information.
    Financial,
    /// Family / next-of-kin / personal-relationship information.
    Family,
}

impl PiiCategory {
    /// Stable lowercase identifier (matches the serde representation). Handy
    /// for logs and for downstream code that keys off the string form.
    pub fn as_str(self) -> &'static str {
        match self {
            PiiCategory::Email => "email",
            PiiCategory::PhoneNumber => "phone_number",
            PiiCategory::NationalId => "national_id",
            PiiCategory::CreditCard => "credit_card",
            PiiCategory::BankAccount => "bank_account",
            PiiCategory::IpAddress => "ip_address",
            PiiCategory::PostalAddress => "postal_address",
            PiiCategory::DateOfBirth => "date_of_birth",
            PiiCategory::Passport => "passport",
            PiiCategory::Medical => "medical",
            PiiCategory::Legal => "legal",
            PiiCategory::Financial => "financial",
            PiiCategory::Family => "family",
        }
    }

    /// Scoring weight contributed when this category is present. Higher =
    /// stronger identification signal. Tuned toward recall — see
    /// [`crate::openhuman::security::pii::detector`] for how weights combine.
    pub(crate) fn weight(self) -> u32 {
        match self {
            // High-confidence direct identifiers — enough to flag on their own.
            PiiCategory::NationalId => 50,
            PiiCategory::CreditCard => 50,
            PiiCategory::Passport => 45,
            PiiCategory::BankAccount => 40,
            // Sensitive topical context.
            PiiCategory::Medical => 30,
            PiiCategory::Legal => 25,
            PiiCategory::Financial => 25,
            // Moderate identifiers.
            PiiCategory::DateOfBirth => 20,
            PiiCategory::PostalAddress => 20,
            PiiCategory::Family => 20,
            // Weak / commonly-shared identifiers.
            PiiCategory::Email => 15,
            PiiCategory::PhoneNumber => 15,
            PiiCategory::IpAddress => 10,
        }
    }

    /// A "strong identifier" is a high-confidence, directly-identifying value
    /// whose mere presence should force [`RiskLevel::High`], independent of the
    /// numeric score. Guards recall against future weight edits.
    pub fn is_strong_identifier(self) -> bool {
        matches!(
            self,
            PiiCategory::NationalId
                | PiiCategory::CreditCard
                | PiiCategory::Passport
                | PiiCategory::BankAccount
        )
    }
}

/// Per-category match tally. Carries the category and how many times it was
/// seen — never the matched text itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryHit {
    /// Which category matched.
    pub category: PiiCategory,
    /// Number of matches for this category across the scanned content.
    pub count: usize,
}

/// Outcome of scanning a piece of content for identification risk.
///
/// This is the public contract S3/S4 (and, after rebase, the S2 egress
/// descriptor) consume. It intentionally holds only the risk level, a numeric
/// score, and the matched categories/counts — no raw values.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PiiScanResult {
    /// Overall identification-risk level.
    pub level: RiskLevel,
    /// Numeric score behind [`level`](Self::level). Exposed for callers that
    /// want a finer-grained ranking than the four-way enum.
    pub score: u32,
    /// Distinct categories detected, sorted and de-duplicated for
    /// deterministic output.
    pub categories: Vec<PiiCategory>,
    /// Per-category match counts, sorted by category.
    pub hits: Vec<CategoryHit>,
}

impl PiiScanResult {
    /// Convenience: content carries at least some identification risk.
    pub fn is_sensitive(&self) -> bool {
        self.level.is_sensitive()
    }

    /// Convenience: was a specific category detected?
    pub fn has_category(&self, category: PiiCategory) -> bool {
        self.categories.contains(&category)
    }
}
