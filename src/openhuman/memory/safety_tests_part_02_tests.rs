use super::*;

/// Parity oracle: the new byte prefilter must be a SUPERSET of the legacy
/// `SCREEN` regex set. For every corpus input, if the old combined set would
/// have matched the normalized text, the new per-class scan must flag at least
/// one class — otherwise a real PII candidate would be silently dropped.
#[test]
fn prefilter_is_superset_of_legacy_screen() {
    use regex::RegexSet;

    // Byte-for-byte the pattern list this PR removed from `pii.rs`.
    let legacy_screen = RegexSet::new([
        r"\d{11,}",
        r"\d{3}\.\d{3}\.\d{3}-\d{2}",
        r"\d{2}\.\d{3}\.\d{3}/\d{4}-\d{2}",
        r"\d{2}-\d{8}-\d",
        r"(?i)[A-Z]{3,4}\d{6}",
        r"(?:マイナンバー|個人番号|My\s?Number)",
        r"\+\d{7}",
        r"\(?[2-9]\d{2}\)?[\s.\-]\d{3}[\s.\-]\d{4}",
        r"\d{3}-\d{2}-\d{4}",
        r"\b[A-Z]{2}\d{2}[A-Z0-9]",
        r"\d{4}[\s\-]\d{4}[\s\-]\d{4}",
        r"(?i)aadhaar|aadhar|आधार|uidai",
        r"(?i)[A-Z]{5}\d{4}[A-Z]",
        r"(?i)[A-Z]{2}\d{6}[A-D]",
        r"\b\d{8}[A-Z]\b",
        r"(?i)[XYZ]\d{7}[A-Z]",
        r"\d{6}-[1-4]\d{6}",
    ])
    .expect("legacy screen");

    let corpus = [
        // Real PII, one per class.
        "CPF: 111.444.777-35.",
        "Sem mascara 11144477735 ok",
        "CNPJ 11.222.333/0001-81",
        "contract 11222333000181 yes",
        "CUIT 20-11111111-2",
        "Mi RFC VECJ880326XK4 .",
        "マイナンバー: 123456789012",
        "個人番号 123456789012",
        "My Number 123456789012",
        // Whitespace-separator variants the precise regexes accept via `\s`.
        "My\tNumber 123456789012",
        "My\nNumber 123456789012",
        "2341\n2341\n2346",
        "12025551234",
        "phone +15551234567",
        "call 415-555-0123 thanks",
        "+1 (212) 555-7890",
        "ssn 123-45-6789",
        "card 4111 1111 1111 1111 thanks",
        "card 378282246310005 used",
        "IBAN DE89370400440532013000 ok",
        "Aadhaar 2341 2341 2346",
        "Aadhaar: 234123412346",
        "आधार 234123412346",
        "uidai 234123412346",
        "PAN: ABCDE1234F",
        "NI no AB123456C",
        "DNI 12345678Z",
        "NIE X1234567L",
        "주민번호 900101-1234567",
        // Scanner-built / borderline identifiers.
        "12025551234-1543890267@g.us:2026-05-30",
        "+12025551234:2026-05-30",
        "accepted:000001747729035001",
        "screen_intelligence_vision-1747729035001-VSCode",
        "Order 123456789012 shipped today.",
        // Clean text (screen won't match; nothing to assert but exercises path).
        "memory/global/preferences",
        "the quick brown fox jumps",
        "just some ordinary words here",
    ];

    for input in corpus {
        let nview = NormalizedView::build(input);
        if legacy_screen.is_match(&nview.normalized) {
            assert!(
                scan_candidates(&nview.normalized).any(),
                "legacy SCREEN matched but new prefilter flagged nothing: {input:?}"
            );
        }
    }
}

#[test]
fn tax_ids_enforce_lengths_checksums_and_repetition_rules() {
    assert!(valid_cpf(&digits("529.982.247-25")));
    assert!(!valid_cpf(&digits("111.111.111-11")));
    assert!(!valid_cpf(&digits("5299822472")));
    assert!(valid_cnpj(&digits("11.222.333/0001-81")));
    assert!(!valid_cnpj(&digits("11.222.333/0001-82")));
    assert!(!valid_cnpj(&digits("00000000000000")));
    assert!(valid_cuit(&digits("20-12345678-6")));
    assert!(!valid_cuit(&digits("20-12345678-7")));
    assert!(!valid_cuit(&digits("2012345678")));
}

#[test]
fn payment_checksums_reject_bad_bounds_and_checksums() {
    assert!(valid_luhn("4111 1111 1111 1111"));
    assert!(!valid_luhn("4111 1111 1111 1112"));
    assert!(!valid_luhn("7992739871"));
    assert!(valid_iban("GB82 WEST 1234 5698 7654 32"));
    assert!(!valid_iban("GB82 WEST 1234 5698 7654 33"));
    assert!(!valid_iban("GB00"));
}

#[test]
fn identity_validators_cover_checksums_reserved_values_and_prefixes() {
    assert!(valid_verhoeff(&digits("234567890124")));
    assert!(!valid_verhoeff(&digits("134567890124")));
    assert!(!valid_verhoeff(&digits("234567890125")));
    assert!(valid_ssn("123-45-6789"));
    assert!(!valid_ssn("666-45-6789"));
    assert!(!valid_ssn("123-00-6789"));
    assert!(!valid_ssn("123-45-0000"));
    assert!(valid_dni_es("12345678Z"));
    assert!(!valid_dni_es("12345678A"));
    assert!(valid_nie_es("X1234567L"));
    assert!(!valid_nie_es("A1234567L"));
    assert!(valid_nino("AA123456A"));
    assert!(!valid_nino("BG123456A"));
    assert!(!valid_nino("DA123456A"));
    assert!(!valid_nino("AA12345A"));
}
