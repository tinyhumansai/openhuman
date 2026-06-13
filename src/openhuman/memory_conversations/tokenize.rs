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
//! explode the dictionary while character bigrams stay tractable. The
//! Gemini Deep Research write-up (see PR description) flagged this as the
//! single most important multilingual mitigation. We therefore generate:
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
//! Implemented in `normalize()` as a single iterator chain. The order
//! matters — strip-marks must run on the decomposed form, lowercase must
//! run on stripped code points, and the final NFKC re-compose unifies
//! compatibility variants for byte-stable indexing/querying:
//!
//! 1. **NFKD** — decompose so combining marks become standalone code points
//!    (Polish ą → `a` + ̨, Arabic kataba+harakat → base letters + marks).
//! 2. **Strip combining marks** — uses `canonical_combining_class` to
//!    drop diacritics across all scripts (Polish ą→a, ć→c; Arabic harakat;
//!    Hebrew niqqud; combining tone marks; etc.) without needing
//!    per-language tables.
//! 3. **Lowercase** — Unicode-aware case folding for cross-alphabet
//!    case insensitivity.
//! 4. **NFKC** — re-compose to canonical form (and unify compatibility
//!    characters: half/full-width CJK variants, Arabic presentation forms,
//!    ligatures) so byte equality lines up at lookup time.
//! 5. **Non-decomposing fold** (`fold_non_decomposing`) — small per-letter
//!    table for decorated letters NFKD leaves untouched (Polish ł, German
//!    ß, Norwegian ø, Icelandic þ/ð, Latin æ/œ, Turkish ı, Croatian đ,
//!    Maltese ħ, Sami ŋ).

use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;

/// Normalize a piece of text for indexing or querying. Idempotent: running
/// the output through `normalize` again yields the same string.
///
/// After the standard NFKD + strip-combining-marks pass we additionally
/// fold a small set of letters that *look* decorated but don't decompose
/// canonically (so NFKD alone leaves them unchanged). Polish ł/Ł is the
/// motivating case — a Polish user typing `lacka` reasonably expects to
/// find `łącka`. Same idea for German ß, Norwegian ø, Icelandic þ/ð and
/// Latin æ.
pub fn normalize(text: &str) -> String {
    let stripped: String = text
        .nfkd() // decompose so combining marks become standalone code points
        .filter(|c| canonical_combining_class(*c) == 0)
        .flat_map(char::to_lowercase)
        .nfkc() // re-compose to canonical form for downstream byte equality
        .collect();
    fold_non_decomposing(&stripped)
}

/// Apply per-letter folds for non-decomposing "decorated" letters that
/// NFKD leaves untouched. Run only after lowercase + NFKD so we don't
/// need uppercase entries.
fn fold_non_decomposing(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'ł' => out.push('l'),
            'ø' => out.push('o'),
            'ß' => out.push_str("ss"),
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            'þ' => out.push_str("th"),
            'ð' => out.push('d'),
            'đ' => out.push('d'),
            'ħ' => out.push('h'),
            'ı' => out.push('i'), // Turkish dotless i
            'ŋ' => out.push('n'),
            other => out.push(other),
        }
    }
    out
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
mod tests {
    use super::*;

    #[test]
    fn normalize_lowercases_ascii() {
        assert_eq!(normalize("Hello World"), "hello world");
    }

    #[test]
    fn normalize_strips_polish_diacritics() {
        // Polish: Kraków, żółć, łąka, plus a Spanish parity case.
        // ł/Ł are *not* canonically decomposable (no combining ogonek
        // form) so we fold them manually via `fold_non_decomposing`.
        assert_eq!(normalize("Kraków"), "krakow");
        assert_eq!(normalize("żółć"), "zolc");
        assert_eq!(normalize("łąka"), "laka");
        assert_eq!(normalize("Mañana"), "manana");
    }

    #[test]
    fn normalize_folds_non_decomposing_letters() {
        // Letters that NFKD leaves untouched but a user typing without
        // the diacritic would still expect to find.
        assert_eq!(normalize("Łódź"), "lodz");
        assert_eq!(normalize("Straße"), "strasse");
        assert_eq!(normalize("Bjørn"), "bjorn");
        assert_eq!(normalize("Þórr"), "thorr");
    }

    #[test]
    fn normalize_strips_arabic_harakat() {
        // Arabic: word "kataba" (he wrote) with harakat marks vs without
        let with_marks = "كَتَبَ";
        let without_marks = "كتب";
        assert_eq!(normalize(with_marks), normalize(without_marks));
    }

    #[test]
    fn normalize_unifies_cjk_halfwidth_fullwidth() {
        // NFKC maps half-width katakana to full-width.
        let halfwidth = "ｶﾀｶﾅ"; // half-width
        let fullwidth = "カタカナ"; // full-width
        assert_eq!(normalize(halfwidth), normalize(fullwidth));
    }

    #[test]
    fn normalize_is_idempotent() {
        let s = "Café — 東京 — żółć";
        let once = normalize(s);
        let twice = normalize(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn ngrams_emits_trigrams_for_latin() {
        let g = ngrams("kitten");
        assert_eq!(g, vec!["kit", "itt", "tte", "ten"]);
    }

    #[test]
    fn ngrams_emits_bigrams_for_cjk() {
        // 日本語 → 日本, 本語
        let g = ngrams("日本語");
        assert_eq!(g, vec!["日本", "本語"]);
    }

    #[test]
    fn ngrams_mixed_script_splits_at_boundary() {
        // "東京tokyo" → CJK run [東京] gives bigram "東京",
        // Latin run [tokyo] gives trigrams tok, oky, kyo.
        let g = ngrams("東京tokyo");
        assert_eq!(g, vec!["東京", "tok", "oky", "kyo"]);
    }

    #[test]
    fn ngrams_drops_runs_too_short() {
        // "ab東" → Latin run [ab] is only 2 chars → dropped; CJK run [東]
        // is only 1 char → dropped. Empty result.
        let g = ngrams("ab東");
        assert!(g.is_empty(), "got {:?}", g);
    }

    #[test]
    fn ngrams_substring_inside_word_is_indexable() {
        // After normalize, "Concatenate" → "concatenate" → trigrams include
        // "cat". This is the canonical substring-inside-word scenario that
        // motivates the character-n-gram scheme over word tokenization.
        let normalized = normalize("Concatenate");
        let g = ngrams(&normalized);
        assert!(g.contains(&"cat"), "trigrams: {:?}", g);
    }

    #[test]
    fn ngrams_empty_input_returns_empty() {
        assert!(ngrams("").is_empty());
    }

    #[test]
    fn is_cjk_classifies_common_scripts() {
        assert!(is_cjk('東'));
        assert!(is_cjk('あ')); // hiragana
        assert!(is_cjk('カ')); // katakana
        assert!(is_cjk('한')); // hangul syllable
        assert!(!is_cjk('a'));
        assert!(!is_cjk('ą'));
        assert!(!is_cjk('，')); // CJK punctuation — intentionally NOT cjk
    }
}
