//! Parse model output into inline completions.

fn normalize_inline_text(value: &str) -> String {
    value
        .replace(['\u{200B}', '\u{200C}', '\u{200D}', '\u{FEFF}'], "")
        .replace(['\u{00A0}', '\u{2028}', '\u{2029}', '\t', '→'], " ")
}

fn trim_generation_prefixes(mut value: &str) -> &str {
    value = value.trim_start();

    // Common wrappers from LLM output formatting.
    for prefix in ["suffix:", "completion:", "result:", "output:"] {
        if value
            .get(..prefix.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(prefix))
        {
            value = value.get(prefix.len()..).unwrap_or(value).trim_start();
            break;
        }
    }

    value
}

fn strip_inline_wrapper_prefix(value: &str) -> &str {
    fn strip_known_markers(input: &str) -> Option<&str> {
        for marker in ["- ", "* ", "> ", "→ "] {
            if let Some(rest) = input.strip_prefix(marker) {
                return Some(rest.trim_start());
            }
        }
        None
    }

    fn strip_numbered_token(input: &str) -> Option<&str> {
        let bytes = input.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == 0 {
            return None;
        }
        let punctuation = bytes.get(i).copied();
        let following_space = bytes.get(i + 1).copied();
        if matches!(punctuation, Some(b'.' | b')')) && following_space == Some(b' ') {
            return input.get(i + 2..).map(str::trim_start);
        }
        None
    }

    let trimmed = value.trim_start();
    if let Some(stripped) = strip_known_markers(trimmed) {
        return stripped;
    }
    if let Some(stripped) = strip_numbered_token(trimmed) {
        return stripped;
    }

    // Quoted marker variants, e.g. "\"- item" or "\"1. item".
    if let Some(after_quote) = trimmed.strip_prefix('"') {
        if let Some(stripped) = strip_known_markers(after_quote) {
            return stripped;
        }
        if let Some(stripped) = strip_numbered_token(after_quote) {
            return stripped;
        }
    }

    trimmed
}

pub(crate) fn sanitize_inline_completion(raw: &str, context: &str) -> String {
    let raw_norm = normalize_inline_text(raw);
    let line = raw_norm
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    if line.is_empty() {
        return String::new();
    }

    let unquoted = line.trim_matches('"');
    let mut cleaned = strip_inline_wrapper_prefix(unquoted).trim().to_string();
    cleaned = trim_generation_prefixes(&cleaned).to_string();

    cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");

    if cleaned.eq_ignore_ascii_case("none") || cleaned.eq_ignore_ascii_case("n/a") {
        return String::new();
    }

    let context_norm = normalize_inline_text(context)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    // Avoid overly aggressive overlap stripping for very short contexts.
    // Example: context="hello", model="hello world" should usually stay as
    // "hello world" instead of collapsing to "world".
    const MIN_CONTEXT_CHARS_FOR_DEDUP: usize = 6;
    let should_dedup_against_context = context_norm.chars().count() >= MIN_CONTEXT_CHARS_FOR_DEDUP;

    if !context_norm.is_empty() && should_dedup_against_context {
        // If model returned full text, keep suffix only.
        if cleaned.starts_with(&context_norm) {
            cleaned = cleaned[context_norm.len()..].trim_start().to_string();
        } else {
            // Remove overlap between end of context and start of prediction.
            let cleaned_chars: Vec<char> = cleaned.chars().collect();
            let max_overlap = context_norm
                .chars()
                .count()
                .min(cleaned_chars.len())
                .min(160);
            for overlap in (1..=max_overlap).rev() {
                let overlap_prefix: String = cleaned_chars.iter().take(overlap).collect();
                if context_norm.ends_with(&overlap_prefix) {
                    cleaned = cleaned_chars
                        .iter()
                        .skip(overlap)
                        .collect::<String>()
                        .trim_start()
                        .to_string();
                    break;
                }
            }
        }

        // If "completion" is already part of the context tail, drop it.
        if !cleaned.is_empty() && context_norm.ends_with(&cleaned) {
            return String::new();
        }
    }

    if cleaned.chars().count() > 96 {
        cleaned = cleaned.chars().take(96).collect();
    }

    cleaned
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
