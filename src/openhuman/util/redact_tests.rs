use super::redact;

#[test]
fn redaction_is_stable_for_the_same_input() {
    assert_eq!(redact("alice@example.com"), redact("alice@example.com"));
}

#[test]
fn redaction_distinguishes_different_inputs() {
    assert_ne!(redact("alice@example.com"), redact("bob@example.com"));
}

#[test]
fn redaction_never_echoes_the_input() {
    let out = redact("alice@example.com");
    assert!(!out.contains("alice"), "{out}");
    assert_eq!(out.len(), 8, "expected 8 hex chars, got {out:?}");
    assert!(out.chars().all(|c| c.is_ascii_hexdigit()), "{out}");
}

/// Pins byte-parity with the `tinymemory_core::util::redact::redact` this
/// replaced at five call sites, so old and new log lines for the same id
/// still match when someone greps across the change.
///
/// The expected value is a **literal**, not a call into the engine helper.
/// Asserting against the engine would reintroduce exactly the dependency
/// this module exists to remove — and `memory::direct_engine_refs_tests`
/// counts inline `#[cfg(test)]` references, so it would also have to be
/// allowlisted. A pinned constant tests the same property without either.
#[test]
fn matches_the_engine_helper_it_replaced() {
    assert_eq!(redact("gmail:alice@example.com"), "c3e9777d");
}
