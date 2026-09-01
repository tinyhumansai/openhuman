//! Normalization + n-gram tokenization tests.

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
    // Letters that have no canonical base but a user typing without
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
    // Half-width katakana maps to full-width.
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

#[test]
fn normalize_folds_fullwidth_ascii() {
    // NFKC folds the full-width ASCII variants; the port must too, or an
    // ASCII query cannot retrieve indexed full-width Latin content.
    assert_eq!(normalize("ＡＢＣ"), "abc");
    assert_eq!(normalize("ｈｅｌｌｏ　ｗｏｒｌｄ"), "hello world");
    assert_eq!(normalize("１２３！"), "123!");
    // Idempotent: the fold lands on plain ASCII, which passes through.
    assert_eq!(normalize(&normalize("ＡＢＣ")), "abc");
}
