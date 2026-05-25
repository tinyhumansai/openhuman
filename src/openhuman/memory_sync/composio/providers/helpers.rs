//! Shared helpers for Composio provider implementations.

/// Helper used by every provider's `fetch_user_profile` impl.
///
/// Walks a JSON object using a list of dotted-path candidates and
/// returns the first non-empty string match. Keeps each provider's
/// extraction code free of repetitive `as_object().and_then(...)`
/// chains.
pub(crate) fn pick_str(value: &serde_json::Value, paths: &[&str]) -> Option<String> {
    for path in paths {
        let mut cur = value;
        let mut ok = true;
        for segment in path.split('.') {
            match cur.get(segment) {
                Some(next) => cur = next,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        if let Some(s) = cur.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}
