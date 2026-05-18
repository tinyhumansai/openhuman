/// Returns true if a 400 response body indicates the user is out of
/// budget / has insufficient balance / over their plan. These are
/// deterministic user-state errors — already surfaced in the UI as a
/// toast — and must not flow to Sentry as errors.
///
/// Match is case-insensitive against any of the known phrases. Keep the
/// list deliberately tight: false positives demote real backend bugs.
pub fn is_budget_exhausted_message(body: &str) -> bool {
    const PHRASES: &[&str] = &[
        "insufficient budget",
        "budget exceeded",
        "add credits",
        "insufficient balance",
    ];

    let lower = body.to_ascii_lowercase();
    PHRASES.iter().any(|phrase| lower.contains(phrase))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_known_budget_exhaustion_phrases() {
        for body in [
            "Insufficient budget",
            "Budget exceeded",
            "Insufficient balance",
            "Add credits to continue",
        ] {
            assert!(
                is_budget_exhausted_message(body),
                "{body:?} must be classified as budget-exhausted user-state"
            );
        }
    }

    #[test]
    fn detection_is_case_insensitive() {
        assert!(is_budget_exhausted_message("INSUFFICIENT BUDGET"));
        assert!(is_budget_exhausted_message("budget EXCEEDED — ADD credits"));
        assert!(is_budget_exhausted_message("Insufficient BALANCE"));
    }

    #[test]
    fn ignores_non_budget_messages() {
        for body in [
            "Bad request: missing field",
            "Invalid request: model not found",
            "HTTP 400 Bad Request",
            "",
        ] {
            assert!(
                !is_budget_exhausted_message(body),
                "{body:?} must not be classified as budget-exhausted"
            );
        }
    }
}
