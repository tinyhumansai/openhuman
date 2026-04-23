//! Title generation helpers for conversation threads.

/// Collapses all runs of whitespace (spaces, tabs, newlines) into a single
/// space and strips leading/trailing whitespace.
pub(crate) fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::collapse_whitespace;

    #[test]
    fn collapse_whitespace_normalizes() {
        assert_eq!(collapse_whitespace("  a   b\tc\nd  "), "a b c d");
    }

    #[test]
    fn collapse_whitespace_empty() {
        assert_eq!(collapse_whitespace(""), "");
    }

    #[test]
    fn collapse_whitespace_only_whitespace() {
        assert_eq!(collapse_whitespace("   \t\n "), "");
    }
}
