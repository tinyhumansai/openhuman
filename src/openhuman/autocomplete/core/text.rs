use super::types::MAX_SUGGESTION_CHARS;

pub(super) fn truncate_tail(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

pub(super) fn sanitize_suggestion(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or_default().trim();
    let cleaned = first_line
        .trim_matches('"')
        .replace('\t', " ")
        .replace('\r', "")
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return String::new();
    }
    truncate_tail(&cleaned, MAX_SUGGESTION_CHARS)
}

pub(super) fn is_no_text_candidate_error(err: &str) -> bool {
    err.contains("ERROR:no_text_candidate_found")
}

pub(super) fn normalize_ax_value(raw: &str) -> String {
    let v = raw.trim();
    if v.eq_ignore_ascii_case("missing value") {
        String::new()
    } else {
        v.to_string()
    }
}

pub(super) fn parse_ax_number(raw: &str) -> Option<i32> {
    let trimmed = normalize_ax_value(raw);
    if trimmed.is_empty() {
        return None;
    }
    let cleaned = trimmed.replace(',', ".");
    cleaned.parse::<f64>().ok().map(|v| v.round() as i32)
}
