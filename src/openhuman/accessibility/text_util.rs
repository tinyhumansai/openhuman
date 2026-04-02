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
