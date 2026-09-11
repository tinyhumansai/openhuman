//! Tests for `credential_ref` parsing, redaction, and the pre-lookup gates.

use super::*;

/// The name used everywhere below. Distinctive enough that a substring search
/// for it cannot pass by accident.
const SECRET_NAME: &str = "supermemory-prod-key-9f3a";

#[test]
fn parses_the_canonical_spelling() {
    let r = CredentialRef::parse("keychain:supermemory").expect("canonical form parses");
    assert_eq!(r.scheme(), CredentialRefScheme::Keychain);
    assert_eq!(r.name(), "supermemory");
}

#[test]
fn trims_whitespace_around_both_halves() {
    // A hand-edited config.toml must resolve the same entry as the canonical
    // spelling rather than looking up a name with a leading space.
    for raw in [
        "  keychain:supermemory  ",
        "keychain: supermemory",
        "keychain :supermemory",
        "\tkeychain:\tsupermemory\n",
    ] {
        let r = CredentialRef::parse(raw).unwrap_or_else(|e| panic!("{raw:?} should parse: {e}"));
        assert_eq!(r.name(), "supermemory", "for input {raw:?}");
        assert_eq!(r.scheme(), CredentialRefScheme::Keychain);
    }
}

#[test]
fn scheme_is_case_insensitive_but_the_name_is_not() {
    for raw in ["KEYCHAIN:Prod", "KeyChain:Prod", "keychain:Prod"] {
        let r = CredentialRef::parse(raw).unwrap_or_else(|e| panic!("{raw:?} should parse: {e}"));
        assert_eq!(r.scheme(), CredentialRefScheme::Keychain);
        // The name is a keychain key and the backing store is case-sensitive,
        // so normalising it would look up the wrong entry.
        assert_eq!(r.name(), "Prod", "name must survive verbatim for {raw:?}");
    }
}

#[test]
fn only_the_first_colon_separates_the_scheme() {
    // A name is free to contain a colon; splitting on every ':' would truncate
    // it and silently resolve a different entry.
    let r = CredentialRef::parse("keychain:ns:sub:key").expect("colons in the name are allowed");
    assert_eq!(r.name(), "ns:sub:key");
}

#[test]
fn rejects_malformed_references() {
    use CredentialRefError::*;
    let cases: &[(&str, CredentialRefError)] = &[
        ("", Empty),
        ("   ", Empty),
        ("supermemory", MissingScheme),
        ("keychain:", EmptyName),
        ("keychain:   ", EmptyName),
        ("vault:supermemory", UnsupportedScheme),
        ("ENV:SUPERMEMORY", UnsupportedScheme),
    ];
    for (raw, expected) in cases {
        let err = CredentialRef::parse(raw).expect_err(&format!("{raw:?} must not parse"));
        assert_eq!(&err, expected, "for input {raw:?}");
    }
}

#[test]
fn an_unsupported_scheme_echoes_back_no_part_of_the_input() {
    // The scheme half is operator-typed text, not fixed vocabulary: a config
    // holding a pasted secret rather than a handle splits into a "scheme" that
    // is a fragment of that secret. Neither half may travel with the error,
    // which is rendered into `subsystems_status`.
    let secret_shaped = "sk-live-abcdef123456";
    for raw in [
        format!("vault:{SECRET_NAME}"),
        format!("{secret_shaped}:tail"),
    ] {
        let rendered = CredentialRef::parse(&raw)
            .expect_err("scheme is not `keychain`")
            .to_string();
        assert!(
            !rendered.contains(SECRET_NAME) && !rendered.contains(secret_shaped),
            "input leaked into the error for {raw:?}: {rendered}"
        );
        assert!(
            !rendered.contains("vault"),
            "the offending scheme is not echoed back: {rendered}"
        );
        // Still actionable: there is exactly one valid scheme, so naming the
        // expectation is the half that helps.
        assert!(
            rendered.contains(KEYCHAIN_SCHEME),
            "the expected scheme should be named: {rendered}"
        );
    }
}

#[test]
fn debug_redacts_the_name() {
    // Deriving Debug here would put the reference into every `{:?}` and panic
    // message; plan-memory.md §7 Tier 3 forbids that.
    let r = CredentialRef::parse(&format!("keychain:{SECRET_NAME}")).expect("parses");
    let rendered = format!("{r:?}");
    assert!(
        !rendered.contains(SECRET_NAME),
        "credential name leaked through Debug: {rendered}"
    );
    assert!(
        rendered.contains("<redacted>"),
        "redaction marker missing: {rendered}"
    );
    // The scheme is still visible — the point is to stay debuggable.
    assert!(
        rendered.contains("Keychain"),
        "scheme should survive: {rendered}"
    );
}

#[test]
fn a_resolved_secret_redacts_the_value_under_debug() {
    // The value this type guards is the secret itself, so a `{:?}` that
    // rendered it would be strictly worse than the credential-name leak the
    // rest of this module exists to prevent.
    const SECRET_VALUE: &str = "sk-live-9f3a-do-not-print-me";

    let resolved = ResolvedSecret::new(SECRET_VALUE.to_string());
    let rendered = format!("{resolved:?}");
    assert!(
        !rendered.contains(SECRET_VALUE),
        "secret leaked through Debug: {rendered}"
    );
    assert!(
        rendered.contains("<redacted>"),
        "redaction marker missing: {rendered}"
    );
    // Still reachable deliberately — redaction must not mean unusable.
    assert_eq!(resolved.expose_secret(), SECRET_VALUE);
}

#[test]
fn the_wrapper_is_needed_because_zeroizing_debug_prints_the_secret() {
    // Guards the premise rather than the fix. `Zeroizing` is a
    // `#[derive(Debug)]` tuple struct, and a derived Debug prints the field
    // whatever its visibility — so returning `Zeroizing<String>` from
    // `resolve` would have rendered the secret at any `{:?}` call site.
    //
    // If this ever fails, zeroize changed its Debug and `ResolvedSecret`'s
    // redaction may have become redundant — which is worth knowing rather
    // than carrying a wrapper whose reason has quietly expired.
    const SECRET_VALUE: &str = "sk-live-9f3a-do-not-print-me";

    let bare = zeroize::Zeroizing::new(SECRET_VALUE.to_string());
    assert!(
        format!("{bare:?}").contains(SECRET_VALUE),
        "zeroize no longer leaks through Debug — re-check whether ResolvedSecret is still needed"
    );
}

#[test]
fn no_error_display_can_carry_a_name_or_a_secret() {
    // This is the property `memory::binding`'s FallbackReason depends on: any
    // of these may be rendered into `subsystems_status`, which is pinned by
    // `fallback_reason_never_contains_credential_ref_or_endpoint`.
    let all = [
        CredentialRefError::Empty,
        CredentialRefError::MissingScheme,
        CredentialRefError::UnsupportedScheme,
        CredentialRefError::EmptyName,
        CredentialRefError::ConsentPending,
        CredentialRefError::ConsentDenied,
        CredentialRefError::Unavailable,
        CredentialRefError::NotFound,
        CredentialRefError::Backend,
    ];
    for err in all {
        let rendered = err.to_string();
        assert!(
            !rendered.contains(SECRET_NAME),
            "{err:?} rendered a credential name: {rendered}"
        );
        assert!(!rendered.is_empty(), "{err:?} rendered an empty message");
    }
}

#[test]
fn preflight_reports_consent_before_availability() {
    // Order is the whole point. With consent unresolved AND no backend, the
    // operator must be told about consent — telling them the keychain is
    // unavailable would send them to fix the wrong thing.
    assert_eq!(
        preflight(PolicyDecision::ConsentRequired, || false),
        Some(CredentialRefError::ConsentPending)
    );
    assert_eq!(
        preflight(PolicyDecision::Declined, || false),
        Some(CredentialRefError::ConsentDenied)
    );
}

#[test]
fn a_declined_prompt_is_not_reported_as_a_pending_one() {
    // `Declined` is an answer, not the absence of one. Reporting it as
    // "pending" tells an operator to wait for a dialog that will never appear.
    let denied = preflight(PolicyDecision::Declined, || true);
    assert_eq!(denied, Some(CredentialRefError::ConsentDenied));
    assert_ne!(denied, Some(CredentialRefError::ConsentPending));

    let pending = preflight(PolicyDecision::ConsentRequired, || true);
    assert_eq!(pending, Some(CredentialRefError::ConsentPending));

    // And the two must not read alike to whoever ends up seeing them.
    assert_ne!(
        denied.map(|e| e.to_string()),
        pending.map(|e| e.to_string()),
        "the declined and pending messages are indistinguishable"
    );
}

#[test]
fn preflight_does_not_probe_the_keychain_once_consent_has_refused() {
    // The real probe's first call is a backend round-trip, so evaluating it
    // eagerly would touch the keychain on exactly the paths consent refused.
    // Taking it as a closure makes that structural; this pins it.
    for decision in [PolicyDecision::ConsentRequired, PolicyDecision::Declined] {
        let probed = std::cell::Cell::new(false);
        let refusal = preflight(decision, || {
            probed.set(true);
            true
        });
        assert!(refusal.is_some(), "{decision:?} must refuse");
        assert!(
            !probed.get(),
            "availability was probed despite {decision:?} refusing first"
        );
    }

    // Conversely, a granted decision must reach the probe — otherwise the gate
    // would be vacuously "ordered" by never running.
    let probed = std::cell::Cell::new(false);
    let _ = preflight(PolicyDecision::Proceed, || {
        probed.set(true);
        true
    });
    assert!(probed.get(), "availability was never probed after Proceed");
}

#[test]
fn preflight_reports_unavailability_only_once_consent_is_granted() {
    assert_eq!(
        preflight(PolicyDecision::Proceed, || false),
        Some(CredentialRefError::Unavailable)
    );
}

#[test]
fn preflight_allows_the_lookup_when_both_gates_pass() {
    assert_eq!(preflight(PolicyDecision::Proceed, || true), None);
}

#[test]
fn preflight_never_reports_a_missing_entry() {
    // NotFound is a fact about the keychain's contents and can only be learned
    // by asking it. A gate that guessed it would report "not configured" for a
    // host that simply has no backend.
    for decision in [
        PolicyDecision::Proceed,
        PolicyDecision::ConsentRequired,
        PolicyDecision::Declined,
    ] {
        for available in [true, false] {
            assert_ne!(
                preflight(decision, || available),
                Some(CredentialRefError::NotFound),
                "preflight invented a NotFound for {decision:?}/available={available}"
            );
        }
    }
}

#[test]
fn keyring_error_kinds_are_fixed_labels_that_carry_no_key() {
    // The classifier exists so a backend failure can be logged without
    // formatting a `KeyringError`, whose `Display` *and* `diagnostic()` both
    // interpolate the key. Every label must therefore be free of it.
    use crate::openhuman::security::keyring::KeyringError;

    let invalid_utf8 = String::from_utf8(vec![0xff]).expect_err("0xff is not utf-8");
    let cases: Vec<(KeyringError, &str)> = vec![
        (
            KeyringError::Os {
                key: SECRET_NAME.to_string(),
                source: ::keyring::Error::NoEntry,
            },
            "os-backend",
        ),
        (
            KeyringError::InvalidUtf8 {
                key: SECRET_NAME.to_string(),
                source: invalid_utf8,
            },
            "invalid-utf8",
        ),
        (KeyringError::Crypto(SECRET_NAME.to_string()), "crypto"),
        (
            KeyringError::VerifyFailed {
                key: SECRET_NAME.to_string(),
            },
            "backend",
        ),
    ];

    for (err, expected) in cases {
        let kind = keyring_error_kind(&err);
        assert_eq!(kind, expected);
        assert!(
            !kind.contains(SECRET_NAME),
            "classification leaked the key: {kind}"
        );
        // Guard the premise: these labels are worth having only because the
        // error's own renderings are unusable here.
        assert!(
            err.to_string().contains(SECRET_NAME) || err.diagnostic().contains(SECRET_NAME),
            "premise broken — {err:?} no longer carries the key, so re-check \
             whether the classifier is still needed"
        );
    }
}

#[test]
fn scheme_round_trips_through_its_wire_spelling() {
    assert_eq!(CredentialRefScheme::Keychain.as_str(), KEYCHAIN_SCHEME);
    let r = CredentialRef::parse(&format!("{KEYCHAIN_SCHEME}:x")).expect("parses");
    assert_eq!(r.scheme().as_str(), KEYCHAIN_SCHEME);
}
