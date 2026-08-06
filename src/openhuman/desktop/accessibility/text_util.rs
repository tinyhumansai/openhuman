//! Shared text utilities for accessibility value parsing.

pub fn truncate_tail(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

pub fn normalize_ax_value(raw: &str) -> String {
    let v = raw.trim();
    if v.eq_ignore_ascii_case("missing value") {
        String::new()
    } else {
        v.to_string()
    }
}

pub fn parse_ax_number(raw: &str) -> Option<i32> {
    let trimmed = normalize_ax_value(raw);
    if trimmed.is_empty() {
        return None;
    }
    let cleaned = trimmed.replace(',', ".");
    cleaned.parse::<f64>().ok().and_then(|v| {
        if !v.is_finite() {
            return None;
        }
        let rounded = v.round();
        if rounded < i32::MIN as f64 || rounded > i32::MAX as f64 {
            return None;
        }
        Some(rounded as i32)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- truncate_tail ---

    #[test]
    fn truncate_tail_shorter_than_max_returns_original() {
        assert_eq!(truncate_tail("hello", 10), "hello");
    }

    #[test]
    fn truncate_tail_exactly_max_returns_original() {
        assert_eq!(truncate_tail("hello", 5), "hello");
    }

    #[test]
    fn truncate_tail_longer_than_max_returns_tail() {
        assert_eq!(truncate_tail("hello", 3), "llo");
    }

    #[test]
    fn truncate_tail_empty_string() {
        assert_eq!(truncate_tail("", 5), "");
    }

    #[test]
    fn truncate_tail_zero_max_returns_empty() {
        assert_eq!(truncate_tail("hello", 0), "");
    }

    #[test]
    fn truncate_tail_multibyte_chars_counts_chars_not_bytes() {
        // "héllo" is 5 chars; last 3 = "llo"
        assert_eq!(truncate_tail("héllo", 3), "llo");
    }

    #[test]
    fn truncate_tail_unicode_emoji_counts_codepoints() {
        // "ab🎉cd" — 5 codepoints; last 3 = "🎉cd"
        assert_eq!(truncate_tail("ab🎉cd", 3), "🎉cd");
    }

    // --- normalize_ax_value ---

    #[test]
    fn normalize_ax_value_trims_whitespace() {
        assert_eq!(normalize_ax_value("  hello  "), "hello");
    }

    #[test]
    fn normalize_ax_value_missing_value_lowercase_returns_empty() {
        assert_eq!(normalize_ax_value("missing value"), "");
    }

    #[test]
    fn normalize_ax_value_missing_value_uppercase_returns_empty() {
        assert_eq!(normalize_ax_value("MISSING VALUE"), "");
    }

    #[test]
    fn normalize_ax_value_mixed_case_missing_value_returns_empty() {
        assert_eq!(normalize_ax_value("Missing Value"), "");
    }

    #[test]
    fn normalize_ax_value_empty_string_returns_empty() {
        assert_eq!(normalize_ax_value(""), "");
    }

    #[test]
    fn normalize_ax_value_only_whitespace_returns_empty() {
        assert_eq!(normalize_ax_value("   "), "");
    }

    #[test]
    fn normalize_ax_value_regular_text_unchanged() {
        assert_eq!(normalize_ax_value("some value"), "some value");
    }

    // --- parse_ax_number ---

    #[test]
    fn parse_ax_number_integer_string() {
        assert_eq!(parse_ax_number("42"), Some(42));
    }

    #[test]
    fn parse_ax_number_negative_integer() {
        assert_eq!(parse_ax_number("-7"), Some(-7));
    }

    #[test]
    fn parse_ax_number_float_rounds_to_nearest() {
        assert_eq!(parse_ax_number("42.4"), Some(42));
        assert_eq!(parse_ax_number("42.6"), Some(43));
    }

    #[test]
    fn parse_ax_number_comma_treated_as_decimal_separator() {
        // Locale-style: "1,5" → 1.5 → rounds to 2
        assert_eq!(parse_ax_number("1,5"), Some(2));
    }

    #[test]
    fn parse_ax_number_missing_value_returns_none() {
        assert_eq!(parse_ax_number("missing value"), None);
    }

    #[test]
    fn parse_ax_number_empty_returns_none() {
        assert_eq!(parse_ax_number(""), None);
    }

    #[test]
    fn parse_ax_number_whitespace_only_returns_none() {
        assert_eq!(parse_ax_number("  "), None);
    }

    #[test]
    fn parse_ax_number_non_numeric_returns_none() {
        assert_eq!(parse_ax_number("abc"), None);
    }

    #[test]
    fn parse_ax_number_nan_returns_none() {
        assert_eq!(parse_ax_number("NaN"), None);
    }

    #[test]
    fn parse_ax_number_infinity_returns_none() {
        assert_eq!(parse_ax_number("inf"), None);
        assert_eq!(parse_ax_number("infinity"), None);
    }

    #[test]
    fn parse_ax_number_zero() {
        assert_eq!(parse_ax_number("0"), Some(0));
    }

    #[test]
    fn parse_ax_number_trims_surrounding_whitespace() {
        assert_eq!(parse_ax_number("  10  "), Some(10));
    }
}
