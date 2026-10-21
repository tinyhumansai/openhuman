//! Regression tests for keyring checks and their non-interactive probe gate.

use super::keyring_with_probe;
use crate::doctor::FindingSeverity;

#[test]
fn noninteractive_macos_keyring_check_skips_before_calling_the_probe() {
    let mut called = false;

    let finding = keyring_with_probe(true, false, || {
        called = true;
        ("host-keyring".to_owned(), true)
    });

    assert!(
        !called,
        "the OS keyring probe can display an interactive prompt"
    );
    assert_eq!(finding.check_id, "secrets/keyring");
    assert_eq!(finding.severity, FindingSeverity::Skipped);
}

#[test]
fn noninteractive_other_platforms_also_avoid_unclassified_keyring_probes() {
    let mut called = false;
    let finding = keyring_with_probe(false, false, || {
        called = true;
        ("os-keyring".to_owned(), true)
    });
    assert!(!called);
    assert_eq!(finding.severity, FindingSeverity::Skipped);
}

#[test]
fn interactive_keyring_check_reports_backend_and_availability() {
    let finding = keyring_with_probe(true, true, || ("encrypted_file".to_owned(), true));
    assert_eq!(finding.severity, FindingSeverity::Pass);
    assert!(finding.message.contains("encrypted_file"));
}
