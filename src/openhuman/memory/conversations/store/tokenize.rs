//! Multilingual normalization + character n-gram generation for the
//! cross-thread search inverted index.
//!
//! ## Why character n-grams (not word tokens)?
//!
//! The cross-thread search must find substrings *inside* words — querying
//! `cat` should return messages containing `concatenate` or `Kotlin`. A
//! whitespace/word-boundary tokenizer fundamentally cannot do that, and
//! it also breaks down for CJK scripts that have no whitespace at all.
//! Character n-grams sidestep both problems.
//!
//! ## Why a hybrid trigram + CJK-bigram scheme?
//!
//! Trigrams strike a good balance between recall and dictionary size for
//! alphabetic scripts (~26³ ≈ 17k Latin trigrams). For CJK scripts the
//! alphabet is tens of thousands of characters, so character trigrams
//! explode the dictionary while character bigrams stay tractable. We
//! therefore generate:
//!
//! - **bigrams** for contiguous runs of CJK characters (Han / Hiragana /
//!   Katakana / Hangul),
//! - **trigrams** for everything else.
//!
//! Tokens from both schemes coexist in the same posting map. As long as
//! query-time tokenization runs the *same* code path, lookups stay
//! consistent.
//!
//! ## Normalization pipeline
//!
//! OpenHuman implemented `normalize()` on top of the `unicode-normalization`
//! crate (NFKD → strip combining marks → lowercase → NFKC). That crate is not
//! a dependency of this crate, so this port reproduces the *observable*
//! behaviour with std-only primitives plus small per-character tables:
//!
//! 1. **Lowercase** — Unicode-aware case folding (`char::to_lowercase`) for
//!    cross-alphabet case insensitivity.
//! 2. **Strip combining marks** — drops standalone combining diacritics across
//!    Latin, Arabic (harakat), and Hebrew (niqqud) ranges so a query without
//!    marks matches content with them.
//! 3. **Strip precomposed Latin diacritics** — a per-letter table maps
//!    precomposed accented Latin letters (Latin-1 Supplement + Latin
//!    Extended-A) back to their base letter (Polish ó→o, ż→z; Spanish ñ→n).
//! 4. **Non-decomposing fold** — small per-letter table for decorated letters
//!    that have no canonical base (Polish ł, German ß, Norwegian ø, Icelandic
//!    þ/ð, Latin æ/œ, Turkish ı, Croatian đ, Maltese ħ, Sami ŋ).
//! 5. **Full-width ASCII → ASCII** — folds the full-width variants
//!    (`ＡＢＣ` → `abc`, U+FF01..=U+FF5E, plus the ideographic space) so an
//!    ASCII query retrieves full-width Latin content, matching NFKC.
//! 6. **Half-width → full-width katakana** — unifies the half-width and
//!    full-width kana forms so byte equality lines up at lookup time.
//!
//! The result is idempotent: re-running `normalize` on its own output is a
//! no-op (no precomposed letters, combining marks, or half-width forms remain).

/// Normalize a piece of text for indexing or querying. Idempotent: running
/// the output through `normalize` again yields the same string.
pub fn normalize(text: &str) -> String {
    // Lowercase first so the per-letter tables below only need lowercase
    // entries. `char::to_lowercase` may expand a single char into several.
    let lowered: String = text.chars().flat_map(char::to_lowercase).collect();
    let mut out = String::with_capacity(lowered.len());
    for c in lowered.chars() {
        if is_combining_mark(c) {
            // Drop diacritics that sit on their own code point (Arabic
            // harakat, Hebrew niqqud, Latin combining marks, …).
            continue;
        }
        if let Some(folded) = fold_non_decomposing(c) {
            out.push_str(folded);
        } else if let Some(base) = strip_latin_diacritic(c) {
            out.push(base);
        } else if let Some(ascii) = fullwidth_to_ascii(c) {
            out.push(ascii);
        } else if let Some(full) = halfwidth_to_fullwidth(c) {
            out.push(full);
        } else {
            out.push(c);
        }
    }
    out
}

/// Full-width ASCII variants (U+FF01..=U+FF5E) → their ASCII originals, plus
/// the ideographic space (U+3000) → an ordinary space.
///
/// These are NFKC compatibility folds — `ＡＢＣ` normalizes to `ABC` under
/// NFKC — and the pipeline this port reproduces ran NFKD→NFKC, so without
/// this arm an ASCII query could not retrieve indexed full-width Latin
/// content (common in CJK text, where full-width forms are produced by the
/// IME). The offset is uniform: full-width `！` (U+FF01) through `～`
/// (U+FF5E) sit exactly `0xFEE0` above `!`..=`~`. Lowercasing has already
/// happened by the time this runs, and `char::to_lowercase` maps full-width
/// `Ａ`→`ａ` within the full-width block, so the subtraction lands on the
/// lowercase ASCII letter directly. Disjoint from the half-width katakana
/// range (U+FF61..) handled below.
fn fullwidth_to_ascii(c: char) -> Option<char> {
    match c {
        '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0),
        '\u{3000}' => Some(' '),
        _ => None,
    }
}

/// Per-letter folds for "decorated" letters that have no canonical
/// decomposition (so stripping combining marks alone leaves them unchanged).
/// Polish ł/Ł is the motivating case — a Polish user typing `lacka`
/// reasonably expects to find `łącka`. Lowercase-only entries (run after
/// `to_lowercase`). Returns `&str` because some fold to multiple letters
/// (ß→ss, æ→ae).
fn fold_non_decomposing(c: char) -> Option<&'static str> {
    Some(match c {
        'ł' => "l",
        'ø' => "o",
        'ß' => "ss",
        'æ' => "ae",
        'œ' => "oe",
        'þ' => "th",
        'ð' => "d",
        'đ' => "d",
        'ħ' => "h",
        'ı' => "i", // Turkish dotless i
        'ŋ' => "n",
        _ => return None,
    })
}

/// Map a precomposed accented Latin letter to its base letter. Lowercase-only
/// (run after `to_lowercase`). Covers Latin-1 Supplement and Latin Extended-A.
fn strip_latin_diacritic(c: char) -> Option<char> {
    Some(match c {
        // Latin-1 Supplement
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        // Latin Extended-A
        'ā' | 'ă' | 'ą' => 'a',
        'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' => 'd',
        'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' => 'h',
        'ĩ' | 'ī' | 'ĭ' | 'į' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' => 'l',
        'ń' | 'ņ' | 'ň' => 'n',
        'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' | 'ŧ' => 't',
        'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        _ => return None,
    })
}

/// Returns true for standalone combining marks that should be dropped during
/// normalization (diacritics that sit on their own code point). Covers the
/// Latin combining block, Arabic harakat, and Hebrew niqqud.
fn is_combining_mark(c: char) -> bool {
    matches!(
        c as u32,
        0x0300..=0x036F      // Combining Diacritical Marks (Latin etc.)
        | 0x0483..=0x0489    // Combining Cyrillic
        | 0x0591..=0x05BD    // Hebrew points
        | 0x05BF
        | 0x05C1 | 0x05C2
        | 0x05C4 | 0x05C5
        | 0x05C7
        | 0x0610..=0x061A    // Arabic signs
        | 0x064B..=0x065F    // Arabic harakat / tashkil
        | 0x0670             // Arabic superscript alef
        | 0x06D6..=0x06DC    // Arabic small high marks
        | 0x06DF..=0x06E4
        | 0x06E7 | 0x06E8
        | 0x06EA..=0x06ED
        | 0xFE20..=0xFE2F    // Combining Half Marks
    )
}

/// Map a half-width katakana code point to its full-width equivalent so the
/// two encodings hash identically. Returns `None` for anything outside the
/// half-width katakana letter block.
fn halfwidth_to_fullwidth(c: char) -> Option<char> {
    // Full-width katakana targets for U+FF66..=U+FF9D, in order.
    const HW_KATAKANA: [u32; 56] = [
        0x30F2, 0x30A1, 0x30A3, 0x30A5, 0x30A7, 0x30A9, 0x30E3, 0x30E5, 0x30E7, 0x30C3, 0x30FC,
        0x30A2, 0x30A4, 0x30A6, 0x30A8, 0x30AA, 0x30AB, 0x30AD, 0x30AF, 0x30B1, 0x30B3, 0x30B5,
        0x30B7, 0x30B9, 0x30BB, 0x30BD, 0x30BF, 0x30C1, 0x30C4, 0x30C6, 0x30C8, 0x30CA, 0x30CB,
        0x30CC, 0x30CD, 0x30CE, 0x30CF, 0x30D2, 0x30D5, 0x30D8, 0x30DB, 0x30DE, 0x30DF, 0x30E0,
        0x30E1, 0x30E2, 0x30E4, 0x30E6, 0x30E8, 0x30E9, 0x30EA, 0x30EB, 0x30EC, 0x30ED, 0x30EF,
        0x30F3,
    ];
    let u = c as u32;
    if (0xFF66..=0xFF9D).contains(&u) {
        return char::from_u32(HW_KATAKANA[(u - 0xFF66) as usize]);
    }
    None
}

/// Returns true for code points that should be tokenized as CJK bigrams.
///
/// Covers Han ideographs (CJK Unified + Ext A + Compatibility), Japanese
/// kana (Hiragana, Katakana), and Hangul (Jamo + precomposed syllables).
/// CJK punctuation and symbols (U+3000..=U+303F) are intentionally
/// excluded — they should be treated as token delimiters, not content.
pub fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x3040..=0x309F  // Hiragana
        | 0x30A0..=0x30FF  // Katakana
        | 0x3400..=0x4DBF  // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF  // CJK Unified Ideographs
        | 0xF900..=0xFAFF  // CJK Compatibility Ideographs
        | 0x1100..=0x11FF  // Hangul Jamo
        | 0xAC00..=0xD7AF  // Hangul Syllables
    )
}

/// Tokenize already-normalized text into character n-grams as borrowed
/// slices into `normalized`.
///
/// Returning `&str` (instead of owned `String`s) keeps the hot search
/// path allocation-free: query-time ngram extraction only needs to look
/// up posting-list keys, never to insert. On the insert side, the index
/// allocates a fresh key only when an ngram is brand-new to the corpus —
/// see `InvertedIndex::insert`.
///
/// - CJK runs (≥2 chars) → bigrams.
/// - Non-CJK runs (≥3 chars) → trigrams.
/// - Runs shorter than the relevant n are dropped (they cannot be
///   substring-matched against any document containing them anyway, so
///   the Phase 2 verification will catch them via the linear fallback in
///   `InvertedIndex::search`).
///
/// Word boundaries inside a run do NOT split the n-gram window — we
/// deliberately want substring matches that span punctuation.
pub fn ngrams(normalized: &str) -> Vec<&str> {
    let mut out = Vec::new();
    // Capture (byte_offset, is_cjk) per char. Byte offsets let us slice
    // `normalized` directly to return `&str` views; the cjk flag drives
    // the script-class run partitioning below.
    let chars: Vec<(usize, bool)> = normalized
        .char_indices()
        .map(|(b, c)| (b, is_cjk(c)))
        .collect();
    if chars.is_empty() {
        return out;
    }
    let end_byte = normalized.len();

    // Walk contiguous runs of "same script class" (CJK vs non-CJK) and
    // emit the appropriate n-gram size for each run.
    let mut i = 0;
    while i < chars.len() {
        let cjk = chars[i].1;
        let mut j = i + 1;
        while j < chars.len() && chars[j].1 == cjk {
            j += 1;
        }
        let n = if cjk { 2 } else { 3 };
        if j - i >= n {
            for k in i..=j - n {
                let start = chars[k].0;
                let end = if k + n < chars.len() {
                    chars[k + n].0
                } else {
                    end_byte
                };
                out.push(&normalized[start..end]);
            }
        }
        i = j;
    }
    out
}

#[cfg(test)]
#[path = "tokenize_tests.rs"]
mod tests;
