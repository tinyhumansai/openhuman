use super::*;

/// An unparseable base is the fail-closed default rather than a panic or a
/// pushable bucket. `otlp_url` returns a non-URL placeholder when no
/// backend host resolves.
#[test]
fn an_unparseable_base_is_external() {
    for base in ["", "not a url", "/api/v1/ingestion"] {
        assert_eq!(
            environment_for_base(&Config::default(), base),
            "external",
            "{base:?} must fail closed"
        );
    }
}
