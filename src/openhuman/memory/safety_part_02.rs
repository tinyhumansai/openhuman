
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
        let overlaps = kept.last().is_some_and(|k| h.start < k.end);
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
//
// A pre-pass that defeats fullwidth-digit and zero-width-char bypasses while
// keeping a byte map back to the original string, so matches found on the
// normalized view can be spliced onto the exact original bytes.

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

// ---------- Byte-oriented candidate pre-filter ----------
//
// Replaces the always-resident combined `RegexSet` (one shared NFA plus a
// per-thread lazy-DFA cache in *every* process/thread) with a single cheap pass
// over the raw bytes. The scan derives per-class candidate flags from a handful
// of structural signals — digit-run lengths, punctuation presence, uppercase /
// alpha presence, `+`, and case-insensitive keyword probes (including the
// non-Latin Aadhaar and My-Number keywords). Each flag then decides whether that
// class's precise validation regex is worth compiling and running; the precise
// `Regex`es stay `LazyLock`, so a class that never sees a candidate is never
// compiled at all. At 100–1000 concurrent agents that turns "combined NFA + N
// thread-local DFA caches resident forever" into "only the regexes a workload
// actually needs, compiled on first hit".
//
// Correctness: every flag is a NECESSARY CONDITION of the class's *precise*
// regex, so a flag can only over-fire (harmless — the precise regex then simply
// fails to match), never under-fire on real PII. Consequently, whenever a
// precise pattern would have matched without the pre-filter, its flag is set and
// it still runs — output is unchanged. The union of the flags is a superset of
// the legacy `SCREEN` set (pinned by `prefilter_is_superset_of_legacy_screen`).
// The NANP phone class gates on the *screen*-entry necessary condition — an
// internal `digit sep digit` separator OR a `\d{11,}` run (the old SCREEN reached
// `PHONE_NANP_RE` through both) — faithfully preserving the documented "a bare
// 10-digit NANP run is never reached" behavior while still redacting a bare
// `1`+10-digit country-code number — see
// `redact_pii_does_not_reach_bare_10_digit_nanp_today`.

/// Per-class candidate flags produced by [`scan_candidates`]. A set flag means
/// "run this class's precise regex"; an unset flag means the class cannot
/// possibly match, so its regex is skipped (and never compiled).
#[derive(Default, Clone, Copy)]
struct Candidates {
    cpf_fmt: bool,
    cnpj_fmt: bool,
    cuit: bool,
    iban: bool,
    cc: bool,
    cnpj_bare: bool,
    cpf_bare: bool,
    aadhaar_fmt: bool,
    aadhaar_kw: bool,
    dni: bool,
    nie: bool,
    nino: bool,
    ssn: bool,
    rrn: bool,
    rfc: bool,
    pan_in: bool,
    phone_e164: bool,
    phone_nanp: bool,
    mynumber: bool,
}

impl Candidates {
    /// True if any class is a candidate — i.e. the text is worth a precise pass.
    fn any(&self) -> bool {
        self.cpf_fmt
            || self.cnpj_fmt
            || self.cuit
            || self.iban
            || self.cc
            || self.cnpj_bare
            || self.cpf_bare
            || self.aadhaar_fmt
            || self.aadhaar_kw
            || self.dni
            || self.nie
            || self.nino
            || self.ssn
            || self.rrn
            || self.rfc
            || self.pan_in
            || self.phone_e164
            || self.phone_nanp
            || self.mynumber
    }
}

/// Case-insensitive (ASCII-only case folding) substring test over raw bytes.
/// Non-ASCII bytes compare exactly, so this also serves as an exact matcher for
/// the multibyte Devanagari / Japanese keyword needles.
fn contains_ci(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if hay.len() < needle.len() {
        return false;
    }
    hay.windows(needle.len())
        .any(|w| w.iter().zip(needle).all(|(a, b)| a.eq_ignore_ascii_case(b)))
}

/// Aadhaar keyword needles — ASCII forms plus Devanagari `आधार`.
const AADHAAR_KEYWORDS: &[&[u8]] = &[b"aadhaar", b"aadhar", b"uidai", b"uid", "आधार".as_bytes()];
/// My-Number Japanese keyword needles. The English `My\s?Number` variant is
/// handled separately (see `scan_candidates`) so any `\s` separator between the
/// two words is recognised, not just a literal space.
const MYNUMBER_JP_KEYWORDS: &[&[u8]] = &["マイナンバー".as_bytes(), "個人番号".as_bytes()];

/// Single linear pass over the bytes deriving every per-class candidate flag.
///
/// Only ASCII structural bytes carry signal here; multibyte UTF-8 lead /
/// continuation bytes are all `>= 0x80`, so scanning `as_bytes()` for ASCII
/// digits/punctuation/letters is boundary-safe. Keyword probes run over the
/// same byte slice so the non-Latin needles match verbatim.
fn scan_candidates(text: &str) -> Candidates {
    let bytes = text.as_bytes();

    let mut total_digits: usize = 0;
    let mut max_digit_run: usize = 0;
    let mut cur_run: usize = 0;
    let mut has_dot = false;
    let mut has_dash = false;
    let mut has_slash = false;
    // Any ASCII whitespace separator (space, tab, newline, CR, form feed,
    // vertical tab). The precise Aadhaar pattern separates its groups with
    // `[\s-]`, which matches the whole `\s` class — so gating on space/tab
    // alone would under-fire on newline-separated Aadhaar (a real PII drop).
    let mut has_ws = false;
    let mut has_upper = false;
    let mut has_alpha = false;
    let mut has_xyz = false;
    let mut has_plus = false;
    // NANP-style "separated group" signal: some `[digit or ')'] [sep] [digit]`
    // window exists (sep ∈ space/tab/./-). This is the necessary condition of
    // the old SCREEN NANP entry, which required internal separators — keeping
    // bare separator-less 10-digit runs out of the phone path.
    let mut nanp_sep = false;

    for (i, &b) in bytes.iter().enumerate() {
        if b.is_ascii_digit() {
            total_digits += 1;
            cur_run += 1;
            if cur_run > max_digit_run {
                max_digit_run = cur_run;
            }
        } else {
            cur_run = 0;
            match b {
                b'.' => has_dot = true,
                b'-' => has_dash = true,
                b'/' => has_slash = true,
                b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => has_ws = true,
                b'+' => has_plus = true,
                b'A'..=b'Z' => {
                    has_upper = true;
                    has_alpha = true;
                    if matches!(b, b'X' | b'Y' | b'Z') {
                        has_xyz = true;
                    }
                }
                b'a'..=b'z' => {
                    has_alpha = true;
                    if matches!(b, b'x' | b'y' | b'z') {
                        has_xyz = true;
                    }
                }
                _ => {}
            }
        }

        if matches!(b, b' ' | b'\t' | b'.' | b'-') && i > 0 && i + 1 < bytes.len() {
            let prev = bytes[i - 1];
            let next = bytes[i + 1];
            if (prev.is_ascii_digit() || prev == b')') && next.is_ascii_digit() {
                nanp_sep = true;
            }
        }
    }

    let has_digit = total_digits > 0;
    let aadhaar_kw = AADHAAR_KEYWORDS.iter().any(|kw| contains_ci(bytes, kw));
    // English `My\s?Number` accepts any single `\s` between the words, so a tab-
    // or newline-separated keyword (`My\tNumber`) must still flag. Requiring both
    // `my` and `number` substrings is a necessary condition of the precise regex
    // and covers every whitespace variant; it may over-fire (harmless — the
    // precise `MYNUM_RE` re-checks the separator and the trailing 12 digits).
    let mynumber = MYNUMBER_JP_KEYWORDS.iter().any(|kw| contains_ci(bytes, kw))
        || (contains_ci(bytes, b"my") && contains_ci(bytes, b"number"));

    let cand = Candidates {
        // Formatted CPF `\d{3}\.\d{3}\.\d{3}-\d{2}` — needs digits, `.`, `-`.
        cpf_fmt: has_digit && has_dot && has_dash,
        // Formatted CNPJ `\d{2}\.\d{3}\.\d{3}/\d{4}-\d{2}` — adds `/`.
        cnpj_fmt: has_digit && has_dot && has_slash && has_dash,
        // CUIT `\d{2}-\d{8}-\d` — needs digits and `-`.
        cuit: has_digit && has_dash,
        // IBAN `[A-Z]{2}\d{2}…` — case-sensitive uppercase letters and digits.
        iban: has_upper && has_digit,
        // Credit card `(?:\d[\s\-]?){13,19}` — at least 13 digits total.
        cc: total_digits >= 13,
        // Bare CNPJ `\d{14}` — a 14-long digit run.
        cnpj_bare: max_digit_run >= 14,
        // Bare CPF `\d{11}` — an 11-long digit run.
        cpf_bare: max_digit_run >= 11,
        // Formatted Aadhaar `\d{4}[\s-]\d{4}[\s-]\d{4}` — 12 digits + a `\s`/dash
        // separator (any ASCII whitespace, matching the precise `[\s-]` class).
        aadhaar_fmt: total_digits >= 12 && (has_ws || has_dash),
        // Keyword-gated Aadhaar — keyword suffices (precise regex checks digits).
        aadhaar_kw,
        // Spain DNI `\d{8}[A-Z]` — 8-run plus a letter.
        dni: max_digit_run >= 8 && has_alpha,
        // Spain NIE `[XYZ]\d{7}[A-Z]` — X/Y/Z, 7-run, letter.
        nie: has_xyz && max_digit_run >= 7 && has_alpha,
        // UK NINO `[A-Z]{2}\d{6}[A-D]` — letters and a 6-run.
        nino: max_digit_run >= 6 && has_alpha,
        // US SSN `\d{3}-\d{2}-\d{4}` — digits and `-`.
        ssn: has_digit && has_dash,
        // Korea RRN `\d{6}-[1-4]\d{6}` — a 6-run and `-`.
        rrn: max_digit_run >= 6 && has_dash,
        // Mexico RFC `[A-ZÑ&]{3,4}\d{6}[A-Z0-9]{3}` — a 6-run (leading class may
        // be all non-ASCII `Ñ`, so gate on the digit run alone, not on letters).
        rfc: max_digit_run >= 6,
        // India PAN `[A-Z]{5}\d{4}[A-Z]` — letters and a 4-run.
        pan_in: max_digit_run >= 4 && has_alpha,
        // E.164 `\+\d{7,15}` — a `+` and a 7+ digit run.
        phone_e164: has_plus && max_digit_run >= 7,
        // NANP — screen-entry necessary condition. The old SCREEN reached
        // `PHONE_NANP_RE` via either the separated-group pattern OR the long
        // `\d{11,}` run (which covers a bare `1`+10-digit country-code number
        // like `12025551234`). A bare 10-digit run still stays out of the phone
        // path (no internal separator, run length 10 < 11).
        phone_nanp: nanp_sep || max_digit_run >= 11,
        // My Number — keyword suffices (precise regex checks the 12 digits).
        mynumber,
    };

    log::trace!(
        "[pii] scan_candidates bytes={} digits={} max_run={} nanp_sep={} any={}",
        bytes.len(),
        total_digits,
        max_digit_run,
        nanp_sep,
        cand.any()
    );

    cand
}

// ---------- Checksum and structural validators for PII candidates ----------

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
    sum.is_multiple_of(10)
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
