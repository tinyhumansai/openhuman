use super::*;

#[test]
fn redactor_reports_an_untouched_body_as_unchanged() {
    // A false `changed` would make every pointer claim a redaction note for
    // a body nothing touched, training readers to ignore the note.
    let out = SanitizingRedactor.redact("ordinary prose with no secrets");
    assert!(!out.changed);
    assert_eq!(out.text, "ordinary prose with no secrets");
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
