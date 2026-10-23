use super::skill_scan::{scan_approval_refusal, scan_bundle_snapshot, unapproved_scan_refusal};
use super::*;
use tinyskills::Verdict as ScanVerdict;

#[test]
fn scan_snapshot_binds_document_and_resource_content() {
    let clean = scan_bundle_snapshot("---\nname: demo\ndescription: safe\n---\nbody", &[]);
    let changed_document =
        scan_bundle_snapshot("---\nname: demo\ndescription: changed\n---\nbody", &[]);
    let changed_resource = scan_bundle_snapshot(
        "---\nname: demo\ndescription: safe\n---\nbody",
        &[("references/guide.md", "resource")],
    );

    assert_ne!(clean.bundle_sha256, changed_document.bundle_sha256);
    assert_ne!(clean.bundle_sha256, changed_resource.bundle_sha256);
}

#[test]
fn scanner_verdict_covers_the_document_and_bundled_resources() {
    let document_block =
        scan_bundle_snapshot("---\nname: demo\ndescription: safe\n---\nbody\u{200b}", &[]);
    let resource_block = scan_bundle_snapshot(
        "---\nname: demo\ndescription: safe\n---\nbody",
        &[("references/guide.md", "safe\u{200b}text")],
    );

    assert_eq!(document_block.verdict, ScanVerdict::Block);
    assert_eq!(resource_block.verdict, ScanVerdict::Block);
}

#[test]
fn only_a_clean_scan_can_run_without_a_security_approval() {
    let clean = scan_bundle_snapshot("ordinary workflow text", &[]);
    let warning = scan_bundle_snapshot(
        "ordinary workflow text",
        &[("scripts/runner.exe", "executable resource")],
    );
    let blocked = scan_bundle_snapshot("text\u{200b}with invisible control", &[]);

    assert_eq!(unapproved_scan_refusal(&clean), None);
    assert_eq!(
        unapproved_scan_refusal(&warning),
        Some("workflow scan warning requires human approval")
    );
    assert_eq!(
        unapproved_scan_refusal(&blocked),
        Some("workflow scan blocked this bundle")
    );
}

#[test]
fn workflow_scan_approval_is_bound_to_the_approved_bundle_digest() {
    let warning = scan_bundle_snapshot(
        "ordinary workflow text",
        &[("scripts/runner.exe", "executable resource")],
    );
    let changed = scan_bundle_snapshot(
        "ordinary workflow text",
        &[("scripts/runner.exe", "changed executable resource")],
    );
    let blocked = scan_bundle_snapshot("text\u{200b}with invisible control", &[]);

    assert_eq!(
        scan_approval_refusal(&warning, None),
        Some("workflow scan warning requires human approval")
    );
    assert_eq!(
        scan_approval_refusal(&warning, Some(warning.bundle_sha256.as_str())),
        None
    );
    assert_eq!(
        scan_approval_refusal(&changed, Some(warning.bundle_sha256.as_str())),
        Some("workflow bundle changed after security approval")
    );
    assert_eq!(
        scan_approval_refusal(&blocked, Some(blocked.bundle_sha256.as_str())),
        Some("workflow scan blocked this bundle")
    );
}
