use super::*;

#[test]
fn diagnostics_omit_secret_key_names() {
    let invalid = SecretStorageError::InvalidUtf8 {
        key: "sentinel-secret-key-name".into(),
        source: String::from_utf8(vec![0xff]).unwrap_err(),
    };
    let verify = SecretStorageError::VerifyFailed {
        key: "sentinel-secret-key-name".into(),
    };
    for diagnostic in [invalid.diagnostic(), verify.diagnostic()] {
        assert!(
            !diagnostic.contains("sentinel-secret-key-name"),
            "{diagnostic}"
        );
        assert!(!diagnostic.contains("u8"));
    }
}
