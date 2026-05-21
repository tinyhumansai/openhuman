//! Multilingual personal-PII redaction (national IDs, financial identifiers,
//! international phone) — on-device, regex + checksum only, zero network.
//!
//! ## Design — security first
//!
//! 1. **Checksum gating where possible.** CPF, CNPJ, CUIT, credit-card (Luhn),
//!    IBAN (mod-97), Aadhaar (Verhoeff), Spanish DNI/NIE (check letter), and
//!    US SSN reserved-range filters all reject look-alikes that aren't real
//!    identifiers. The false-positive rate from format alone is too high; the
//!    checksums bring it back to acceptable.
//!
//! 2. **Bypass-resistant.** Inputs are run through [`normalize`] before
//!    matching, which:
//!      - strips zero-width characters (U+200B/200C/200D/FEFF/2060/180E),
//!      - folds fullwidth digits (`０-９` → `0-9`) and fullwidth `．－／：`
//!        to their ASCII counterparts,
//!      - folds Arabic-Indic and Eastern Arabic-Indic digits to ASCII.
//!    Match offsets are mapped back to the original text so we only redact
//!    the bytes that actually carry PII; surrounding text is untouched.
//!
//! 3. **Overlap-safe.** Patterns are run in priority order; later matches
//!    that overlap an earlier redaction are dropped, so a credit-card span
//!    can't also be partially matched as a phone number.
//!
//! 4. **Out of scope.** Contextual PII (`"call me at the usual number"`),
//!    compound PII (`name + employer + city`), arbitrary names, and freeform
//!    dates-of-birth all require NER/LLM and are NOT addressed here. This
//!    module is honest about its scope.

use once_cell::sync::Lazy;
use regex::{Regex, RegexSet};

use super::{SanitizationReport, Sanitized};

// ---------- Replacement tokens ----------

const PII_RFC: &str = "[REDACTED_PII_RFC]";
const PII_CPF: &str = "[REDACTED_PII_CPF]";
const PII_CNPJ: &str = "[REDACTED_PII_CNPJ]";
const PII_CUIT: &str = "[REDACTED_PII_CUIT]";
const PII_MYNUM: &str = "[REDACTED_PII_MYNUMBER]";
const PII_PHONE: &str = "[REDACTED_PII_PHONE]";
const PII_SSN: &str = "[REDACTED_PII_SSN]";
const PII_CC: &str = "[REDACTED_PII_CREDIT_CARD]";
const PII_IBAN: &str = "[REDACTED_PII_IBAN]";
const PII_AADHAAR: &str = "[REDACTED_PII_AADHAAR]";
const PII_PAN_IN: &str = "[REDACTED_PII_PAN_IN]";
const PII_NINO: &str = "[REDACTED_PII_NINO]";
const PII_DNI: &str = "[REDACTED_PII_DNI]";
const PII_RRN: &str = "[REDACTED_PII_RRN]";

// ---------- Patterns ----------

// Brazilian CPF, formatted: NNN.NNN.NNN-NN
static CPF_FMT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b\d{3}\.\d{3}\.\d{3}-\d{2}\b").expect("cpf fmt"));
// Brazilian CPF, bare: 11 consecutive digits. Checksum-gated; ~1% raw FP.
static CPF_BARE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{11}\b").expect("cpf bare"));

// Brazilian CNPJ, formatted: NN.NNN.NNN/NNNN-NN
static CNPJ_FMT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b\d{2}\.\d{3}\.\d{3}/\d{4}-\d{2}\b").expect("cnpj fmt"));
// Brazilian CNPJ, bare: 14 consecutive digits.
static CNPJ_BARE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{14}\b").expect("cnpj bare"));

// Argentine CUIT/CUIL: NN-NNNNNNNN-N (formatted only — bare 11-digit with
// single check digit has ~9% FP on random IDs, too noisy without context).
static CUIT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{2}-\d{8}-\d\b").expect("cuit"));

// Mexican RFC: 3-4 letters (incl. Ñ &) + 6 digits + 3 alphanumeric homoclave.
static RFC_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b[A-ZÑ&]{3,4}\d{6}[A-Z0-9]{3}\b").expect("rfc"));

// Japan My Number (12 digits) gated by a Japanese or English keyword within
// ~30 chars. Bare 12-digit runs without keyword are too noisy.
static MYNUM_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:マイナンバー|個人番号|My\s?Number)[\s:はがを、.\-]{0,12}(\d{12})\b")
        .expect("my number")
});

// E.164 phone: + followed by 7-15 digits, no separators.
static PHONE_E164_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\+\d{7,15}\b").expect("e164"));

// NANP (US/Canada) formatted phone. Area code must start 2-9; first digit of
// central-office code also 2-9 (real NANP rule).
static PHONE_NANP_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(?:\+?1[\s.\-]?)?\(?([2-9]\d{2})\)?[\s.\-]?([2-9]\d{2})[\s.\-]?(\d{4})\b")
        .expect("nanp phone")
});

// US SSN: NNN-NN-NNNN. Range filter applied below.
static SSN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").expect("ssn"));

// Credit card: 13-19 digits with optional spaces/dashes every 4. Luhn-gated.
static CC_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b(?:\d[\s\-]?){13,19}\b").expect("credit card"));

// IBAN: 2 letter country code + 2 check digits + 11-30 alphanumeric.
// Allow optional spaces every 4 chars (common human format).
static IBAN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b[A-Z]{2}\d{2}(?:[\s]?[A-Z0-9]){11,30}\b").expect("iban"));

// India Aadhaar: 4-4-4 digit groups (space or hyphen) OR contiguous 12 digits
// gated by keyword. Verhoeff-checksum-gated when grouped, keyword-gated when
// bare (Verhoeff alone has ~10% raw FP rate on random 12-digit runs).
static AADHAAR_FMT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b\d{4}[\s\-]\d{4}[\s\-]\d{4}\b").expect("aadhaar formatted"));
static AADHAAR_KW_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:aadhaar|aadhar|आधार|uidai|uid)[\s:#\-no.]{0,10}(\d{12})\b")
        .expect("aadhaar keyword")
});

// India PAN: 5 letters, 4 digits, 1 letter. Very high signal — no checksum.
static PAN_IN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b[A-Z]{5}\d{4}[A-Z]\b").expect("pan-in"));

// UK NINO: 2 letters + 6 digits + suffix A/B/C/D.
static NINO_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b[A-Z]{2}\d{6}[A-D]\b").expect("nino"));

// Spain DNI: 8 digits + check letter. NIE: starts X/Y/Z, then 7 digits + letter.
static DNI_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b\d{8}[A-Z]\b").expect("dni"));
static NIE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b[XYZ]\d{7}[A-Z]\b").expect("nie"));

// South Korea RRN: NNNNNN-CXXXXXX where C is gender/century digit (1-4).
static RRN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{6}-[1-4]\d{6}\b").expect("rrn"));

// Cheap whole-text pre-filter so we skip the per-pattern scans entirely on
// PII-free text. Each entry roughly corresponds to one of the patterns above.
static SCREEN: Lazy<RegexSet> = Lazy::new(|| {
    RegexSet::new([
        r"\d{11,}",                               // any long digit run → CPF/CNPJ/CC/Aadhaar/IBAN
        r"\d{3}\.\d{3}\.\d{3}-\d{2}",             // CPF
        r"\d{2}\.\d{3}\.\d{3}/\d{4}-\d{2}",       // CNPJ
        r"\d{2}-\d{8}-\d",                        // CUIT
        r"(?i)[A-Z]{3,4}\d{6}",                   // RFC / general alphanumeric ID
        r"(?:マイナンバー|個人番号|My\s?Number)", // JP keyword
        r"\+\d{7}",                               // E.164
        r"\(?[2-9]\d{2}\)?[\s.\-]\d{3}[\s.\-]\d{4}", // NANP (parens optional)
        r"\d{3}-\d{2}-\d{4}",                     // SSN
        r"\b[A-Z]{2}\d{2}[A-Z0-9]",               // IBAN prefix
        r"\d{4}[\s\-]\d{4}[\s\-]\d{4}",           // Aadhaar formatted
        r"(?i)aadhaar|aadhar|आधार|uidai",         // Aadhaar keyword
        r"(?i)[A-Z]{5}\d{4}[A-Z]",                // PAN-IN
        r"(?i)[A-Z]{2}\d{6}[A-D]",                // NINO
        r"\b\d{8}[A-Z]\b",                        // DNI
        r"(?i)[XYZ]\d{7}[A-Z]",                   // NIE
        r"\d{6}-[1-4]\d{6}",                      // RRN
    ])
    .expect("screen regex set")
});

// ---------- Public API ----------

/// Redact format-based multilingual PII from `text`.
///
/// Runs a Unicode normalization pre-pass to defeat fullwidth-digit and
/// zero-width-char bypasses. Match indices from the normalized form are
/// translated back to original byte offsets so only the PII bytes are
/// replaced — surrounding text (including any preserved fullwidth glyphs)
/// is untouched.
pub fn redact_pii(text: &str) -> Sanitized<String> {
    let mut report = SanitizationReport::default();

    // Fast path: no candidate at all.
    if !SCREEN.is_match(text) {
        // Even the screen might miss normalized inputs; check normalized too.
        let nview = NormalizedView::build(text);
        if !SCREEN.is_match(&nview.normalized) {
            return Sanitized {
                value: text.to_string(),
                report,
            };
        }
        return splice_redactions(
            text,
            &nview,
            collect_redactions(&nview.normalized),
            &mut report,
        );
    }

    let nview = NormalizedView::build(text);
    let redactions = collect_redactions(&nview.normalized);
    splice_redactions(text, &nview, redactions, &mut report)
}

/// True if `value` looks like it carries any PII. Used to *reject*
/// namespace/key inputs at boundary checks (analogous to
/// [`super::has_likely_secret`]).
///
/// Uses the **strict** match set — only formatted / keyword-gated patterns.
/// Bare-numeric patterns whose only gate is a checksum (credit card via Luhn,
/// bare CPF, bare CNPJ) are excluded here because their false-positive rate
/// against random digit runs (millisecond timestamps, sequence IDs, padded
/// counters) is too high to use as a hard rejection signal on internal
/// identifiers. Content scrubbing via [`redact_pii`] still applies those
/// patterns — false positives are tolerable there because they only replace
/// bytes inside a string, not reject the whole write.
pub fn has_likely_pii(value: &str) -> bool {
    let nview = NormalizedView::build(value);
    SCREEN.is_match(&nview.normalized) && !collect_strict_redactions(&nview.normalized).is_empty()
}

// ---------- Match collection ----------

#[derive(Debug)]
struct Hit {
    start: usize, // byte offset in NORMALIZED text
    end: usize,
    token: &'static str,
}

fn collect_redactions(norm: &str) -> Vec<Hit> {
    collect_redactions_inner(norm, true)
}

/// Variant of [`collect_redactions`] that omits bare-numeric checksum
/// patterns (credit card via Luhn, bare CPF, bare CNPJ). Used for
/// boundary checks like [`has_likely_pii`] where rejection on a checksum
/// hit alone would have too many false positives on internal identifiers
/// (timestamps, padded counters).
fn collect_strict_redactions(norm: &str) -> Vec<Hit> {
    collect_redactions_inner(norm, false)
}

fn collect_redactions_inner(norm: &str, include_bare_checksum: bool) -> Vec<Hit> {
    let mut hits: Vec<Hit> = Vec::new();

    // Priority order: most specific / highest-confidence first.
    push_checksum(&mut hits, norm, &CPF_FMT_RE, PII_CPF, |s| {
        valid_cpf(digits(s).as_slice())
    });
    push_checksum(&mut hits, norm, &CNPJ_FMT_RE, PII_CNPJ, |s| {
        valid_cnpj(digits(s).as_slice())
    });
    push_checksum(&mut hits, norm, &CUIT_RE, PII_CUIT, |s| {
        valid_cuit(digits(s).as_slice())
    });

    // IBAN before credit card: CC can match an IBAN tail of all digits.
    push_checksum(&mut hits, norm, &IBAN_RE, PII_IBAN, |s| valid_iban(s));

    if include_bare_checksum {
        // Credit card before bare CPF/CNPJ to avoid catching a 13-19 digit run as CPF/CNPJ.
        push_checksum(&mut hits, norm, &CC_RE, PII_CC, |s| valid_luhn(s));

        push_checksum(&mut hits, norm, &CNPJ_BARE_RE, PII_CNPJ, |s| {
            valid_cnpj(digits(s).as_slice())
        });
        push_checksum(&mut hits, norm, &CPF_BARE_RE, PII_CPF, |s| {
            valid_cpf(digits(s).as_slice())
        });
    }

    push_checksum(&mut hits, norm, &AADHAAR_FMT_RE, PII_AADHAAR, |s| {
        valid_verhoeff(digits(s).as_slice())
    });
    // Keyword-gated Aadhaar redacts only the captured 12-digit group.
    push_captured(&mut hits, norm, &AADHAAR_KW_RE, PII_AADHAAR, |digits_str| {
        valid_verhoeff(digits(digits_str).as_slice())
    });

    push_checksum(&mut hits, norm, &DNI_RE, PII_DNI, |s| valid_dni_es(s));
    push_checksum(&mut hits, norm, &NIE_RE, PII_DNI, |s| valid_nie_es(s));
    push_checksum(&mut hits, norm, &NINO_RE, PII_NINO, |s| valid_nino(s));
    push_checksum(&mut hits, norm, &SSN_RE, PII_SSN, |s| valid_ssn(s));
    push_simple(&mut hits, norm, &RRN_RE, PII_RRN);
    push_simple(&mut hits, norm, &RFC_RE, PII_RFC);
    push_simple(&mut hits, norm, &PAN_IN_RE, PII_PAN_IN);

    // Phones: E.164 first (more specific), then NANP.
    push_simple(&mut hits, norm, &PHONE_E164_RE, PII_PHONE);
    push_simple(&mut hits, norm, &PHONE_NANP_RE, PII_PHONE);

    // My Number — captured digit group only, keyword remains visible.
    push_captured(&mut hits, norm, &MYNUM_RE, PII_MYNUM, |_| true);

    dedupe_overlaps(&mut hits);
    hits
}

fn push_simple(hits: &mut Vec<Hit>, norm: &str, re: &Regex, token: &'static str) {
    for m in re.find_iter(norm) {
        hits.push(Hit {
            start: m.start(),
            end: m.end(),
            token,
        });
    }
}

fn push_checksum(
    hits: &mut Vec<Hit>,
    norm: &str,
    re: &Regex,
    token: &'static str,
    ok: impl Fn(&str) -> bool,
) {
    for m in re.find_iter(norm) {
        if ok(m.as_str()) {
            hits.push(Hit {
                start: m.start(),
                end: m.end(),
                token,
            });
        }
    }
}

fn push_captured(
    hits: &mut Vec<Hit>,
    norm: &str,
    re: &Regex,
    token: &'static str,
    ok: impl Fn(&str) -> bool,
) {
    for caps in re.captures_iter(norm) {
        let Some(group) = caps.get(1) else { continue };
        if ok(group.as_str()) {
            hits.push(Hit {
                start: group.start(),
                end: group.end(),
                token,
            });
        }
    }
}

// Sort by start asc, length desc. Then walk in order, dropping any hit whose
// range overlaps a kept hit. Result: earlier + longer wins; no double-redact.
fn dedupe_overlaps(hits: &mut Vec<Hit>) {
    hits.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then((b.end - b.start).cmp(&(a.end - a.start)))
    });
    let mut kept: Vec<Hit> = Vec::with_capacity(hits.len());
    for h in hits.drain(..) {
        let overlaps = kept.last().map_or(false, |k| h.start < k.end);
        if !overlaps {
            kept.push(h);
        }
    }
    *hits = kept;
}

// Splice redactions (whose indices reference NORMALIZED text) back into the
// ORIGINAL text via NormalizedView's byte-offset mapping. This preserves
// non-PII original bytes verbatim (including fullwidth glyphs the user
// intentionally typed) while still scrubbing detected PII.
fn splice_redactions(
    original: &str,
    nview: &NormalizedView,
    hits: Vec<Hit>,
    report: &mut SanitizationReport,
) -> Sanitized<String> {
    if hits.is_empty() {
        return Sanitized {
            value: original.to_string(),
            report: *report,
        };
    }
    let mut out = String::with_capacity(original.len());
    let mut cursor = 0;
    for h in &hits {
        let start_orig = nview.norm_to_orig(h.start);
        let end_orig = nview.norm_to_orig(h.end);
        if start_orig < cursor || start_orig > original.len() || end_orig > original.len() {
            continue;
        }
        out.push_str(&original[cursor..start_orig]);
        out.push_str(h.token);
        cursor = end_orig;
    }
    out.push_str(&original[cursor..]);
    report.pii_redactions += hits.len();
    Sanitized {
        value: out,
        report: *report,
    }
}

// ---------- Unicode normalization for matching ----------

struct NormalizedView {
    normalized: String,
    // For each byte offset i in `normalized`, `byte_map[i]` is the byte offset
    // in the original string where the corresponding char *starts*.
    // The last entry maps the normalized length to the original length, so
    // `norm_to_orig(normalized.len())` is well-defined.
    byte_map: Vec<usize>,
}

impl NormalizedView {
    fn build(original: &str) -> Self {
        let mut normalized = String::with_capacity(original.len());
        let mut byte_map: Vec<usize> = Vec::with_capacity(original.len() + 1);
        for (idx, ch) in original.char_indices() {
            if is_zero_width(ch) {
                continue;
            }
            let mapped = fold_char(ch);
            let start = normalized.len();
            normalized.push(mapped);
            // One byte_map entry per byte of the normalized char.
            let added = normalized.len() - start;
            for _ in 0..added {
                byte_map.push(idx);
            }
        }
        byte_map.push(original.len());
        Self {
            normalized,
            byte_map,
        }
    }

    fn norm_to_orig(&self, norm_byte: usize) -> usize {
        if norm_byte >= self.byte_map.len() {
            return *self.byte_map.last().unwrap_or(&0);
        }
        self.byte_map[norm_byte]
    }
}

fn is_zero_width(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'
            | '\u{200C}'
            | '\u{200D}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{2060}'
            | '\u{180E}'
            | '\u{FEFF}'
    )
}

fn fold_char(c: char) -> char {
    match c {
        // Fullwidth digits 0-9
        '\u{FF10}'..='\u{FF19}' => char::from_u32(c as u32 - 0xFF10 + 0x30).unwrap_or(c),
        // Arabic-Indic digits ٠-٩
        '\u{0660}'..='\u{0669}' => char::from_u32(c as u32 - 0x0660 + 0x30).unwrap_or(c),
        // Eastern Arabic-Indic digits ۰-۹
        '\u{06F0}'..='\u{06F9}' => char::from_u32(c as u32 - 0x06F0 + 0x30).unwrap_or(c),
        // Common fullwidth punctuation we care about for PII formats
        '\u{FF0D}' => '-',
        '\u{FF0E}' => '.',
        '\u{FF0F}' => '/',
        '\u{FF1A}' => ':',
        '\u{2010}'..='\u{2015}' => '-', // various unicode hyphens/dashes
        '\u{2212}' => '-',              // minus sign
        other => other,
    }
}

// ---------- Checksum helpers ----------

fn digits(s: &str) -> Vec<u32> {
    s.chars()
        .filter(|c| c.is_ascii_digit())
        .map(|c| c.to_digit(10).expect("ascii digit"))
        .collect()
}

fn valid_cpf(d: &[u32]) -> bool {
    if d.len() != 11 || d.iter().all(|x| *x == d[0]) {
        return false;
    }
    let s1: u32 = (0..9).map(|i| d[i] * (10 - i as u32)).sum();
    let dv1 = (s1 * 10) % 11 % 10;
    if dv1 != d[9] {
        return false;
    }
    let s2: u32 = (0..10).map(|i| d[i] * (11 - i as u32)).sum();
    let dv2 = (s2 * 10) % 11 % 10;
    dv2 == d[10]
}

fn valid_cnpj(d: &[u32]) -> bool {
    if d.len() != 14 || d.iter().all(|x| *x == d[0]) {
        return false;
    }
    let w1: [u32; 12] = [5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2];
    let s1: u32 = (0..12).map(|i| d[i] * w1[i]).sum();
    let r1 = s1 % 11;
    let dv1 = if r1 < 2 { 0 } else { 11 - r1 };
    if dv1 != d[12] {
        return false;
    }
    let w2: [u32; 13] = [6, 5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2];
    let s2: u32 = (0..13).map(|i| d[i] * w2[i]).sum();
    let r2 = s2 % 11;
    let dv2 = if r2 < 2 { 0 } else { 11 - r2 };
    dv2 == d[13]
}

fn valid_cuit(d: &[u32]) -> bool {
    if d.len() != 11 {
        return false;
    }
    let w: [u32; 10] = [5, 4, 3, 2, 7, 6, 5, 4, 3, 2];
    let s: u32 = (0..10).map(|i| d[i] * w[i]).sum();
    let r = s % 11;
    let dv = match r {
        0 => 0,
        1 => return false,
        _ => 11 - r,
    };
    dv == d[10]
}

// Luhn — used for credit-card validation.
fn valid_luhn(s: &str) -> bool {
    let d = digits(s);
    if d.len() < 13 || d.len() > 19 {
        return false;
    }
    let mut sum = 0u32;
    let mut alt = false;
    for x in d.iter().rev() {
        let v = if alt {
            let doubled = x * 2;
            if doubled > 9 {
                doubled - 9
            } else {
                doubled
            }
        } else {
            *x
        };
        sum += v;
        alt = !alt;
    }
    sum % 10 == 0
}

// IBAN mod-97. Steps: strip spaces, move first 4 chars to end, expand letters
// (A=10..Z=35), divide as a big-integer mod 97, require remainder == 1.
fn valid_iban(s: &str) -> bool {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() < 15 || cleaned.len() > 34 {
        return false;
    }
    if !cleaned.chars().take(2).all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    if !cleaned[2..4].chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let rotated: String = cleaned[4..].chars().chain(cleaned[..4].chars()).collect();
    let mut remainder: u64 = 0;
    for c in rotated.chars() {
        let chunk = if let Some(d) = c.to_digit(10) {
            d as u64
        } else if c.is_ascii_alphabetic() {
            (c.to_ascii_uppercase() as u64) - ('A' as u64) + 10
        } else {
            return false;
        };
        // Expand into the running remainder digit-by-digit so we never need
        // u128. Each letter contributes 2 decimal digits.
        if chunk >= 10 {
            remainder = (remainder * 100 + chunk) % 97;
        } else {
            remainder = (remainder * 10 + chunk) % 97;
        }
    }
    remainder == 1
}

// Verhoeff — used for Aadhaar.
const VERHOEFF_D: [[u8; 10]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    [1, 2, 3, 4, 0, 6, 7, 8, 9, 5],
    [2, 3, 4, 0, 1, 7, 8, 9, 5, 6],
    [3, 4, 0, 1, 2, 8, 9, 5, 6, 7],
    [4, 0, 1, 2, 3, 9, 5, 6, 7, 8],
    [5, 9, 8, 7, 6, 0, 4, 3, 2, 1],
    [6, 5, 9, 8, 7, 1, 0, 4, 3, 2],
    [7, 6, 5, 9, 8, 2, 1, 0, 4, 3],
    [8, 7, 6, 5, 9, 3, 2, 1, 0, 4],
    [9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
];
const VERHOEFF_P: [[u8; 10]; 8] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    [1, 5, 7, 6, 2, 8, 3, 0, 9, 4],
    [5, 8, 0, 3, 7, 9, 6, 1, 4, 2],
    [8, 9, 1, 6, 0, 4, 3, 5, 2, 7],
    [9, 4, 5, 3, 1, 2, 6, 8, 7, 0],
    [4, 2, 8, 6, 5, 7, 3, 9, 0, 1],
    [2, 7, 9, 3, 8, 0, 6, 4, 1, 5],
    [7, 0, 4, 6, 9, 1, 3, 2, 5, 8],
];

fn valid_verhoeff(d: &[u32]) -> bool {
    if d.len() != 12 {
        return false;
    }
    // Aadhaar can't start with 0 or 1.
    if d[0] < 2 {
        return false;
    }
    let mut c: u8 = 0;
    for (i, digit) in d.iter().rev().enumerate() {
        c = VERHOEFF_D[c as usize][VERHOEFF_P[i % 8][*digit as usize] as usize];
    }
    c == 0
}

// US SSN reserved/invalid ranges per SSA.
fn valid_ssn(s: &str) -> bool {
    let d = digits(s);
    if d.len() != 9 {
        return false;
    }
    let area = d[0] * 100 + d[1] * 10 + d[2];
    let group = d[3] * 10 + d[4];
    let serial = d[5] * 1000 + d[6] * 100 + d[7] * 10 + d[8];
    if area == 0 || area == 666 || area >= 900 {
        return false;
    }
    if group == 0 || serial == 0 {
        return false;
    }
    true
}

// Spain DNI check letter — 8 digits mod 23 indexes into a fixed letter table.
const DNI_LETTERS: &[u8; 23] = b"TRWAGMYFPDXBNJZSQVHLCKE";

fn valid_dni_es(s: &str) -> bool {
    let upper = s.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() != 9 {
        return false;
    }
    let num_str = &upper[..8];
    let letter = bytes[8];
    let Ok(num) = num_str.parse::<u32>() else {
        return false;
    };
    DNI_LETTERS[(num % 23) as usize] == letter
}

fn valid_nie_es(s: &str) -> bool {
    let upper = s.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() != 9 {
        return false;
    }
    let prefix = match bytes[0] {
        b'X' => 0u32,
        b'Y' => 1,
        b'Z' => 2,
        _ => return false,
    };
    let Ok(rest) = std::str::from_utf8(&bytes[1..8]) else {
        return false;
    };
    let Ok(num) = rest.parse::<u32>() else {
        return false;
    };
    let composed = prefix * 10_000_000 + num;
    DNI_LETTERS[(composed % 23) as usize] == bytes[8]
}

// UK NINO reserved-prefix blacklist.
fn valid_nino(s: &str) -> bool {
    let upper = s.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() != 9 {
        return false;
    }
    // First char cannot be D F I Q U V; second cannot be D F I O Q U V.
    let bad_first = b"DFIQUV";
    let bad_second = b"DFIOQUV";
    if bad_first.contains(&bytes[0]) || bad_second.contains(&bytes[1]) {
        return false;
    }
    // Reserved two-letter prefixes.
    let reserved = ["BG", "GB", "KN", "NK", "NT", "TN", "ZZ"];
    let prefix = &upper[..2];
    if reserved.contains(&prefix) {
        return false;
    }
    true
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    fn redacts(input: &str, token: &str) {
        let out = redact_pii(input);
        assert!(
            out.value.contains(token),
            "expected {token} in output. input={input:?} output={out:?}"
        );
    }

    fn unchanged(input: &str) {
        let out = redact_pii(input);
        assert_eq!(
            out.value, input,
            "expected no change; report={:?}",
            out.report
        );
        assert_eq!(out.report.pii_redactions, 0);
    }

    // --- CPF ---
    #[test]
    fn cpf_formatted_valid_redacted() {
        redacts("CPF: 111.444.777-35.", PII_CPF);
    }
    #[test]
    fn cpf_formatted_invalid_kept() {
        unchanged("CPF 111.444.777-99 nope");
    }
    #[test]
    fn cpf_all_same_digits_rejected() {
        unchanged("Test 111.111.111-11");
    }
    #[test]
    fn cpf_bare_valid_redacted() {
        redacts("Sem mascara 11144477735 ok", PII_CPF);
    }

    // --- CNPJ ---
    #[test]
    fn cnpj_formatted_valid_redacted() {
        redacts("CNPJ 11.222.333/0001-81", PII_CNPJ);
    }
    #[test]
    fn cnpj_bare_valid_redacted() {
        redacts("contract 11222333000181 yes", PII_CNPJ);
    }

    // --- CUIT ---
    #[test]
    fn cuit_valid_redacted() {
        redacts("CUIT 20-11111111-2", PII_CUIT);
    }
    #[test]
    fn cuit_invalid_kept() {
        unchanged("noise 20-12345678-0 noise");
    }

    // --- RFC ---
    #[test]
    fn rfc_redacted() {
        redacts("Mi RFC VECJ880326XK4 .", PII_RFC);
    }
    #[test]
    fn rfc_lowercase_redacted() {
        redacts("rfc vecj880326xk4", PII_RFC);
    }

    // --- My Number ---
    #[test]
    fn my_number_redacted_with_keyword() {
        redacts("マイナンバー: 123456789012", PII_MYNUM);
    }
    #[test]
    fn bare_12_digits_without_keyword_kept() {
        unchanged("Order 123456789012 shipped today.");
    }

    // --- E.164 + NANP phone ---
    #[test]
    fn e164_redacted() {
        redacts("phone +15551234567", PII_PHONE);
    }
    #[test]
    fn nanp_formatted_redacted() {
        redacts("call 415-555-0123 thanks", PII_PHONE);
    }
    #[test]
    fn nanp_with_country_code_redacted() {
        redacts("+1 (212) 555-7890", PII_PHONE);
    }
    #[test]
    fn nanp_invalid_area_code_kept() {
        unchanged("score 115-555-0123 ish");
    }

    // --- SSN ---
    #[test]
    fn ssn_valid_redacted() {
        redacts("ssn 123-45-6789", PII_SSN);
    }
    #[test]
    fn ssn_reserved_area_kept() {
        unchanged("test 666-12-3456");
    }
    #[test]
    fn ssn_zero_serial_kept() {
        unchanged("test 123-45-0000");
    }

    // --- Credit card / Luhn ---
    #[test]
    fn credit_card_visa_redacted() {
        // Visa test number with valid Luhn.
        redacts("card 4111 1111 1111 1111 thanks", PII_CC);
    }
    #[test]
    fn credit_card_amex_redacted() {
        redacts("card 378282246310005 used", PII_CC);
    }
    #[test]
    fn credit_card_invalid_luhn_kept() {
        unchanged("invoice 4111 1111 1111 1112");
    }

    // --- IBAN ---
    #[test]
    fn iban_de_redacted() {
        // Known test IBAN with valid mod-97.
        redacts("IBAN DE89370400440532013000 ok", PII_IBAN);
    }
    #[test]
    fn iban_invalid_kept() {
        unchanged("noise DE89370400440532013001 noise");
    }

    // --- Aadhaar ---
    #[test]
    fn aadhaar_formatted_verhoeff_valid_redacted() {
        // 234123412346 is a known Verhoeff-valid Aadhaar test number.
        redacts("Aadhaar 2341 2341 2346", PII_AADHAAR);
    }
    #[test]
    fn aadhaar_keyword_bare_redacted() {
        redacts("Aadhaar: 234123412346", PII_AADHAAR);
    }
    #[test]
    fn aadhaar_invalid_verhoeff_kept() {
        unchanged("Random 2341 2341 2345 nope");
    }

    // --- PAN-IN ---
    #[test]
    fn pan_in_redacted() {
        redacts("PAN: ABCDE1234F", PII_PAN_IN);
    }

    // --- NINO ---
    #[test]
    fn nino_redacted() {
        redacts("NI no AB123456C", PII_NINO);
    }
    #[test]
    fn nino_reserved_prefix_kept() {
        unchanged("BG123456A");
    }

    // --- DNI / NIE ---
    #[test]
    fn dni_es_redacted() {
        redacts("DNI 12345678Z", PII_DNI);
    }
    #[test]
    fn dni_es_bad_letter_kept() {
        unchanged("ID 12345678A code");
    }
    #[test]
    fn nie_es_redacted() {
        redacts("NIE X1234567L", PII_DNI);
    }

    // --- RRN Korea ---
    #[test]
    fn rrn_kr_redacted() {
        redacts("주민번호 900101-1234567", PII_RRN);
    }
    #[test]
    fn rrn_kr_bad_gender_digit_kept() {
        unchanged("ref 900101-5234567 nope");
    }

    // --- Bypass resistance ---
    #[test]
    fn fullwidth_digits_cannot_bypass_cpf() {
        // 111.444.777-35 with fullwidth digits and punctuation.
        let input = "CPF: １１１．４４４．７７７－３５ done";
        let out = redact_pii(input);
        assert!(out.value.contains(PII_CPF), "got {out:?}");
    }

    #[test]
    fn zero_width_chars_cannot_bypass_ssn() {
        // U+200B inserted between digits.
        let input = "ssn 1\u{200B}23-4\u{200B}5-6789 done";
        let out = redact_pii(input);
        assert!(out.value.contains(PII_SSN), "got {out:?}");
    }

    #[test]
    fn arabic_indic_digits_normalize_for_phone() {
        let input = "phone +١٥٥٥١٢٣٤٥٦٧";
        let out = redact_pii(input);
        assert!(out.value.contains(PII_PHONE), "got {out:?}");
    }

    // --- Aggressive mix end-to-end ---
    #[test]
    fn aggressive_mixed_document() {
        let input = "\
Cliente RFC VECJ880326XK4. \
Empresa CNPJ 11.222.333/0001-81. \
Argentino CUIT 20-11111111-2. \
Brasileiro CPF 111.444.777-35. \
マイナンバー: 123456789012. \
SSN 123-45-6789. \
Card 4111 1111 1111 1111. \
IBAN DE89370400440532013000. \
PAN ABCDE1234F. \
NI AB123456C. \
DNI 12345678Z. \
RRN 900101-1234567. \
Phone +15551234567.";
        let out = redact_pii(input);
        for token in [
            PII_RFC, PII_CNPJ, PII_CUIT, PII_CPF, PII_MYNUM, PII_SSN, PII_CC, PII_IBAN, PII_PAN_IN,
            PII_NINO, PII_DNI, PII_RRN, PII_PHONE,
        ] {
            assert!(
                out.value.contains(token),
                "missing {token} in: {}",
                out.value
            );
        }
        assert!(out.report.pii_redactions >= 13);
    }

    // --- has_likely_pii ---
    #[test]
    fn has_likely_pii_detects_cpf() {
        assert!(has_likely_pii("user/111.444.777-35"));
    }
    #[test]
    fn has_likely_pii_quiet_on_normal_text() {
        assert!(!has_likely_pii("memory/global/preferences"));
    }

    /// Regression: zero-padded millisecond-timestamp keys must NOT be
    /// flagged as PII even when the digit run happens to satisfy Luhn.
    /// `redact_pii` content scrubbing may still flag the same string —
    /// `has_likely_pii` (used for boundary rejection of internal keys)
    /// must stay strict to formatted/keyword PII only.
    #[test]
    fn has_likely_pii_ignores_bare_luhn_timestamp_keys() {
        // 18-digit padded timestamps where the digit total mod 10 == 0
        // (the Luhn-passing case that previously rejected autocomplete
        // KV writes and screen-intelligence document writes).
        for key in [
            "accepted:000001747729035001",
            "completion:000001747729035011",
            "screen_intelligence_vision-1747729035001-VSCode",
        ] {
            assert!(
                !has_likely_pii(key),
                "internal key {key:?} must not be rejected as PII"
            );
        }
    }

    /// Strict boundary check should still reject formatted PII even though
    /// it skips bare-numeric checksum patterns.
    #[test]
    fn has_likely_pii_still_blocks_formatted_secrets() {
        assert!(has_likely_pii("ssn-123-45-6789"));
        assert!(has_likely_pii("cliente-RFC-VECJ880326XK4"));
        assert!(has_likely_pii("cuit-20-11111111-2"));
    }

    #[test]
    fn empty_text_is_noop() {
        unchanged("");
    }
}
