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
        // abacus's out-of-credits 400 wording (TAURI-RUST-D6X): the managed
        // route-llm account is exhausted. The full body is
        // `"You have no remaining credits to use the LLM apis."`. Anchored on
        // the "no remaining credits" fragment (not the broader "remaining
        // credits", which a positive "you have N remaining credits" balance
        // message could trip) to keep the list tight per the rule above.
        "no remaining credits",
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

    /// Verbatim abacus out-of-credits 400 body (Sentry TAURI-RUST-D6X). The
    /// classifier feeds the native_chat demotion, the `expected_error_kind`
    /// re-report demotion, the `is_budget_event` before_send net, AND the cron
    /// scheduler's terminal billing-halt (`is_budget_exhausted_failure`), so
    /// pinning the exact wire body makes an abacus phrasing drift fail CI
    /// rather than silently re-flood Sentry / re-fire the retry loop.
    #[test]
    fn detects_abacus_no_remaining_credits_400_body() {
        let body = "abacus API error (400 Bad Request): \
            {\"success\": false, \"error\": \"You have no remaining credits to use the LLM apis.\"}";
        assert!(
            is_budget_exhausted_message(body),
            "abacus no-remaining-credits 400 must classify as budget-exhausted user-state"
        );
        // Case-insensitive on the same phrase.
        assert!(is_budget_exhausted_message(
            "You have NO REMAINING CREDITS left"
        ));
    }

    #[test]
    fn ignores_non_budget_messages() {
        for body in [
            "Bad request: missing field",
            "Invalid request: model not found",
            "HTTP 400 Bad Request",
            // A positive-balance message must NOT be demoted: we anchor on the
            // "no remaining credits" fragment precisely so this doesn't trip.
            "You have 100 remaining credits this month",
            "",
        ] {
            assert!(
                !is_budget_exhausted_message(body),
                "{body:?} must not be classified as budget-exhausted"
            );
        }
    }
}
