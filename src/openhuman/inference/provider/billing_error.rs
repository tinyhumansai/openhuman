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
        // Anthropic's BYO-key out-of-credits 400 wording (TAURI-RUST-4MM):
        // the full body is `"Your credit balance is too low to access the
        // Anthropic API. Please go to Plans & Billing to upgrade or purchase
        // credits."` (direct provider, "anthropic API error", not the managed
        // "OpenHuman API error"). Anchored on the "credit balance is too low"
        // fragment — the "too low" qualifier keeps a positive-balance message
        // (e.g. "your credit balance is $50") from tripping, per the tight-list
        // rule above. OpenHuman has no lever over a third-party Anthropic
        // account's balance; budget toast already surfaced in the UI.
        "credit balance is too low",
    ];

    let lower = body.to_ascii_lowercase();
    PHRASES.iter().any(|phrase| lower.contains(phrase))
}

#[cfg(test)]
#[path = "billing_error_tests.rs"]
mod tests;
