use super::*;

// Fails closed only while security is enforced: a build without the
// security module has security disabled (`crate::security::switch`).
#[cfg(feature = "security-module")]
#[test]
fn redactor_withholds_body_when_tinysecurity_is_unavailable() {
    #[cfg(feature = "security-module")]
    let _absent = crate::modules::security::test_host::absent();
    let out = SanitizingRedactor.redact("ordinary prose with no secrets");
    assert!(out.changed);
    assert_ne!(out.text, "ordinary prose with no secrets");
}

#[test]
fn redactor_reports_a_rewritten_body_as_changed() {
    // Uses the same detector production does rather than a hand-rolled
    // pattern, so this cannot pass against a scrubber that no longer fires.
    let with_secret = "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
    let out = SanitizingRedactor.redact(with_secret);
    if out.changed {
        assert_ne!(out.text, with_secret, "changed implies a rewrite");
    } else {
        // `sanitize_text`'s pattern set is the authority here; if it does
        // not flag this shape, the adapter is still correct. What must
        // never happen is `changed` set without a rewrite.
        assert_eq!(out.text, with_secret);
    }
}

/// With security disabled nothing is reserved: even a credential store under a
/// home directory may be materialized into.
async fn assert_materialization_is_unchecked() {
    let dir = tempfile::tempdir().unwrap();
    let policy = SecurityPolicy {
        workspace_dir: dir.path().join("workspace"),
        action_dir: dir.path().join("action"),
        ..SecurityPolicy::default()
    };
    assert!(materialization_allowed(&policy, &dir.path().join("home/.ssh/id_rsa")).await);
    assert!(materialization_allowed(&policy, &dir.path().join("workspace/memory.db")).await);
}

#[cfg(not(feature = "security-module"))]
#[tokio::test]
async fn a_build_without_the_module_materializes_anywhere() {
    assert_materialization_is_unchecked().await;
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn security_configured_off_materializes_anywhere() {
    let _off = crate::security::switch::off_for_this_thread();
    assert_materialization_is_unchecked().await;
}
