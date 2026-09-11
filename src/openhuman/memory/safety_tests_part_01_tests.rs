use super::*;

#[test]
fn sanitize_text_redacts_bearer_and_openai_key() {
    let input = "Authorization: Bearer abcdefghijklmnop and sk-1234567890123456789012345";
    let sanitized = sanitize_text(input);
    assert!(sanitized.value.contains("Bearer [REDACTED]"));
    assert!(!sanitized.value.contains("sk-1234567890123456789012345"));
    assert!(sanitized.report.text_redactions >= 2);
}

#[test]
fn sanitize_text_blocks_private_key_blocks() {
    let input = private_key_fixture("PRIVATE KEY", "abc");
    let sanitized = sanitize_text(&input);
    assert!(sanitized.value.contains(REDACTED_PRIVATE_KEY));
    assert!(sanitized.report.blocked_secret_hits >= 1);
}

#[test]
fn sanitize_json_redacts_sensitive_keys_and_nested_strings() {
    let input = json!({
        "token": "abc123",
        "nested": { "notes": "Bearer supersecretvalue", "ok": "hello" },
        "arr": ["sk-1234567890123456789012345", "safe"]
    });
    let sanitized = sanitize_json(&input);
    assert_eq!(sanitized.value["token"], json!(REDACTED_SECRET));
    assert_eq!(sanitized.value["nested"]["ok"], json!("hello"));
    assert!(sanitized.value["nested"]["notes"]
        .as_str()
        .unwrap_or_default()
        .contains("[REDACTED]"));
    assert!(sanitized.report.key_redactions >= 1);
    assert!(sanitized.report.text_redactions >= 2);
}

#[test]
fn sanitize_json_redacts_common_sensitive_key_variants() {
    let input = json!({
        "db_password": "p@ss", "secret_key": "abc123",
        "api_secret": "def456", "monkey": "banana"
    });
    let sanitized = sanitize_json(&input);
    assert_eq!(sanitized.value["db_password"], json!(REDACTED_SECRET));
    assert_eq!(sanitized.value["secret_key"], json!(REDACTED_SECRET));
    assert_eq!(sanitized.value["api_secret"], json!(REDACTED_SECRET));
    assert_eq!(sanitized.value["monkey"], json!(REDACTED_SECRET));
    assert!(sanitized.report.key_redactions >= 4);
}

#[test]
fn has_likely_secret_detects_common_patterns() {
    assert!(has_likely_secret("api_key=abc123"));
    assert!(has_likely_secret("Bearer abcdefghijklmnopqrstuvwxyz"));
    let slack_token = format!("{}{}-1234567890-abcdef-ghijklmnop", "xo", "xb");
    assert!(has_likely_secret(&slack_token));
    assert!(has_likely_secret("glpat-aaaaaaaaaaaaaaaaaaaa"));
    assert!(has_likely_secret("SG.aaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbb"));
    assert!(!has_likely_secret("I prefer rust"));
}

#[test]
fn sanitize_text_redacts_more_provider_secrets() {
    let input = "auth=Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ== \
                 stripe=sk_live_12345678901234567890 npm=npm_abcdefghijklmnopqrstuvwxyz";
    let sanitized = sanitize_text(input);
    assert!(!sanitized.value.contains("sk_live_12345678901234567890"));
    assert!(!sanitized.value.contains("npm_abcdefghijklmnopqrstuvwxyz"));
    assert!(sanitized.value.contains("[REDACTED]"));
    assert!(sanitized.report.text_redactions >= 2);
}

#[test]
fn sanitize_text_redacts_oauth_url_style_params() {
    let input = "https://example.com/callback\
        ?access_token=abcd1234&refresh_token=efgh5678&id_token=jwt";
    let sanitized = sanitize_text(input);
    assert!(!sanitized.value.contains("abcd1234"));
    assert!(!sanitized.value.contains("efgh5678"));
    assert!(!sanitized.value.contains("id_token=jwt"));
    assert!(sanitized.report.text_redactions >= 3);
}

#[test]
fn sanitize_text_redacts_multiline_private_key_blocks() {
    let key_kind = format!("{} PRIVATE KEY", "OPENSSH");
    let input = format!(
        "BEGIN\n{}\nEND",
        private_key_fixture(&key_kind, "line1\nline2")
    );
    let sanitized = sanitize_text(&input);
    assert!(!sanitized.value.contains(&key_kind));
    assert!(sanitized.value.contains(REDACTED_PRIVATE_KEY));
    assert!(sanitized.report.blocked_secret_hits >= 1);
}

#[test]
fn sanitize_text_also_redacts_pii_after_secrets() {
    let input = "Token sk-abcdefghijklmnopqrstuvwxyz; CPF 111.444.777-35; phone +15551234567";
    let sanitized = sanitize_text(input);
    assert!(!sanitized.value.contains("sk-abcdefghijklmnopqrstuvwxyz"));
    assert!(!sanitized.value.contains("111.444.777-35"));
    assert!(!sanitized.value.contains("+15551234567"));
    assert!(sanitized.value.contains("[REDACTED_PII_CPF]"));
    assert!(sanitized.value.contains("[REDACTED_PII_PHONE]"));
    assert!(sanitized.report.text_redactions >= 1);
    assert_eq!(sanitized.report.pii_redactions, 2);
}

#[test]
fn sanitize_json_propagates_pii_redaction_into_nested_strings() {
    let input = json!({
        "note": "Cliente RFC VECJ880326XK4 confirmado",
        "meta": { "cuit": "20-11111111-2" }
    });
    let sanitized = sanitize_json(&input);
    assert!(sanitized.value["note"]
        .as_str()
        .unwrap_or_default()
        .contains("[REDACTED_PII_RFC]"));
    assert!(sanitized.value["meta"]["cuit"]
        .as_str()
        .unwrap_or_default()
        .contains("[REDACTED_PII_CUIT]"));
    assert!(sanitized.report.pii_redactions >= 2);
}

#[test]
fn sanitize_json_redacts_values_beyond_max_depth() {
    let mut nested = json!("leaf");
    for _ in 0..(MAX_JSON_SANITIZE_DEPTH + 2) {
        nested = json!({ "nested": nested });
    }
    let sanitized = sanitize_json(&nested);
    assert!(sanitized.report.depth_redactions >= 1);
    assert!(sanitized
        .value
        .to_string()
        .contains(&format!("\"{REDACTED_SECRET}\"")));
}

#[test]
fn has_likely_pii_strict_boundary_flags_formatted_national_ids() {
    // The write-rejection boundary is the *strict* set: formatted national IDs
    // only. Bare-numeric / phone-shaped runs and email are excluded (too many
    // false positives against scanner-built identifiers); they are still
    // scrubbed by content redaction.
    assert!(has_likely_pii("ssn 123-45-6789"));
    assert!(has_likely_pii("CPF 111.444.777-35"));
    assert!(has_likely_pii("cliente RFC VECJ880326XK4"));
    assert!(!has_likely_pii("call +15551234567")); // phone: content-scrub only
    assert!(!has_likely_pii("contact alice@example.com")); // email: out of scope
    assert!(!has_likely_pii("just a normal note"));
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
#[test]
fn my_number_keyword_tab_separator_redacted() {
    // `My\s?Number` accepts any single `\s`; the byte prefilter must recognise a
    // tab-separated keyword, not just a literal space.
    redacts("My\tNumber 123456789012", PII_MYNUM);
}
#[test]
fn my_number_keyword_newline_separator_redacted() {
    redacts("My\nNumber 123456789012", PII_MYNUM);
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
#[test]
fn nanp_bare_country_code_redacted() {
    // Separator-less `1`+10-digit NANP: the old SCREEN reached PHONE_NANP_RE via
    // the `\d{11,}` run; the prefilter must keep gating this through the phone
    // class (the bare-CPF checksum rejects it, so nothing else redacts it).
    redacts("12025551234", PII_PHONE);
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
#[test]
fn aadhaar_formatted_newline_separator_redacted() {
    // AADHAAR_FMT_RE separates groups with `[\s-]`; a newline-separated Aadhaar
    // (no keyword, no dash) must still flag the formatted class in the prefilter.
    redacts("2341\n2341\n2346", PII_AADHAAR);
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
fn has_likely_email_detects_email_without_changing_boundary_pii() {
    assert!(has_likely_email("user/alice@example.com"));
    assert!(!has_likely_pii("user/alice@example.com"));
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

/// Regression for Sentry TAURI-RUST-54T / GH #2848: scanner-built
/// `namespace` and `key` values containing bare-numeric phone-shaped
/// digit runs (WhatsApp group JID `<phone>-<unix>@g.us`, WhatsApp
/// broadcast `<phone>@broadcast`, US-prefixed WhatsApp 1:1 JID,
/// telegram numeric peer ID) must NOT be rejected by the boundary
/// PII check. NANP matches `\d{10,11}` with optional separators —
/// strict mode must skip it. Content scrubbing via `redact_pii`
/// continues to redact these substrings (see
/// `redact_pii_still_blurs_bare_phone_in_content` below).
#[test]
fn has_likely_pii_ignores_scanner_bare_phone_keys() {
    for key in [
        // WhatsApp group JID — chat_id = "<phone>-<unix-ts>@g.us"
        "12025551234-1543890267@g.us:2026-05-30",
        // WhatsApp broadcast list
        "12025551234@broadcast:2026-05-30",
        // WhatsApp 1:1 JID, country-coded US number (`1` + 10 digits)
        "12025551234@c.us:2026-05-30",
        // Same shape carried in the namespace
        "whatsapp-web:12025551234@c.us",
        "whatsapp-web:12025551234-1543890267@g.us",
        // Telegram numeric peer_id key
        "4123456789:2026-05-30",
    ] {
        assert!(
            !has_likely_pii(key),
            "scanner-built key {key:?} must not be rejected as PII"
        );
    }
}

/// Same regression but for the E.164 (`+`-prefixed) shape — iMessage
/// posts `key = format!("{chat_id}:{day}")` where `chat_id` can be
/// `+12025551234`. Strict mode must skip; content redaction stays.
#[test]
fn has_likely_pii_ignores_bare_e164_phone_keys() {
    for key in [
        "+12025551234:2026-05-30",
        "imessage:+12025551234",
        "imessage:+12025551234:2026-05-30",
    ] {
        assert!(
            !has_likely_pii(key),
            "E.164-shaped key {key:?} must not be rejected as PII"
        );
    }
}

/// `redact_pii` (content scrubbing path — NOT the boundary check)
/// must still redact formatted NANP and E.164 phone numbers found
/// inside document bodies. False positives in the content path only
/// blur substring bytes; they do not reject the write — which is the
/// asymmetry this PR preserves vs. the boundary check.
///
/// Note: bare 10-digit NANP runs (`2025551234` with no separators)
/// are NOT reached by `redact_pii` at all — the SCREEN fast-path
/// requires either `\d{11,}`, a separator, or `+`, so a bare 10-digit
/// run short-circuits as "no candidate". That pre-existed this PR; a
/// pinning sentinel for it lives below.
#[test]
fn redact_pii_still_blurs_formatted_and_e164_phone_in_content() {
    let out = redact_pii("call me at 202-555-1234 or +12025551234");
    let n_phone = out.value.matches(PII_PHONE).count();
    assert!(
        n_phone >= 2,
        "redact_pii must still blur both formatted NANP and E.164 phones in content, \
             got {n_phone} PII_PHONE token(s) in: {}",
        out.value
    );
    assert!(out.report.pii_redactions >= 2);
}

/// Sentinel pinning a pre-existing SCREEN limitation: a bare 10-digit
/// NANP run (`2025551234` with no separators) is short-circuited by
/// the `SCREEN` fast-path because no `SCREEN` regex matches a 10-digit
/// bare run (`\d{11,}` is the closest, but it needs 11+). This is the
/// status quo on `main` — this PR does not change it. The test exists
/// so any future widening of `SCREEN` (e.g. to catch bare NANP) trips
/// here as a deliberate review checkpoint, NOT a regression.
#[test]
fn redact_pii_does_not_reach_bare_10_digit_nanp_today() {
    let out = redact_pii("call me at 2025551234 thanks");
    assert!(
        !out.value.contains(PII_PHONE),
        "SCREEN fast-path historically skips bare 10-digit NANP — \
             if this test fails, SCREEN was widened; revisit the boundary-check \
             behavior in `has_likely_pii` before adjusting. Got: {}",
        out.value
    );
}

#[test]
fn empty_text_is_noop() {
    unchanged("");
}

// --- Byte prefilter: per-class positives (incl. non-Latin) ---

/// Devanagari Aadhaar keyword must still route into the keyword-gated Aadhaar
/// path (the `आधार` needle lives in `AADHAAR_KEYWORDS`).
#[test]
fn aadhaar_devanagari_keyword_redacted() {
    redacts("आधार 234123412346", PII_AADHAAR);
}

/// Japanese My-Number keyword (kanji form) routes into the My-Number path.
#[test]
fn my_number_kanji_keyword_redacted() {
    redacts("個人番号 123456789012", PII_MYNUM);
}

/// `scan_candidates` flags the right class for representative per-class inputs.
#[test]
fn scan_flags_expected_classes() {
    assert!(scan_candidates("111.444.777-35").cpf_fmt);
    assert!(scan_candidates("11.222.333/0001-81").cnpj_fmt);
    assert!(scan_candidates("20-11111111-2").cuit);
    assert!(scan_candidates("DE89370400440532013000").iban);
    assert!(scan_candidates("4111111111111111").cc);
    assert!(scan_candidates("11222333000181").cnpj_bare);
    assert!(scan_candidates("11144477735").cpf_bare);
    assert!(scan_candidates("2341 2341 2346").aadhaar_fmt);
    assert!(scan_candidates("aadhaar 234123412346").aadhaar_kw);
    assert!(scan_candidates("आधार 234123412346").aadhaar_kw);
    assert!(scan_candidates("12345678Z").dni);
    assert!(scan_candidates("X1234567L").nie);
    assert!(scan_candidates("AB123456C").nino);
    assert!(scan_candidates("123-45-6789").ssn);
    assert!(scan_candidates("900101-1234567").rrn);
    assert!(scan_candidates("VECJ880326XK4").rfc);
    assert!(scan_candidates("ABCDE1234F").pan_in);
    assert!(scan_candidates("+15551234567").phone_e164);
    assert!(scan_candidates("415-555-0123").phone_nanp);
    assert!(scan_candidates("マイナンバー 123456789012").mynumber);
    assert!(scan_candidates("My Number 123456789012").mynumber);
}

/// Clean, PII-free text flags no class at all — the whole precise pass is
/// skipped and every precise regex stays uncompiled.
#[test]
fn scan_clean_text_flags_nothing() {
    for clean in [
        "",
        "just some ordinary words here",
        "memory/global/preferences",
        "the quick brown fox",
        "https://example.com/path?q=1",
        "snake_case_identifier_v2",
    ] {
        let cand = scan_candidates(clean);
        assert!(!cand.any(), "clean text flagged a class: {clean:?}");
    }
}

/// A bare separator-less 10-digit run must NOT flag the NANP phone class — this
/// is what preserves the documented "bare 10-digit NANP is never reached"
/// behavior even though the precise NANP regex would otherwise match it.
#[test]
fn scan_bare_10_digit_run_does_not_flag_nanp() {
    assert!(!scan_candidates("call me at 2025551234 thanks").phone_nanp);
}
