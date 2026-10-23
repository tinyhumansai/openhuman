use super::HTTP_SERVER_COMPILED_IN;

/// Pins the constant to the gate rather than to a hardcoded value: the
/// assertion inverts with the feature, so it holds for both the default
/// build and the slim (`--no-default-features`) build.
#[test]
fn reports_the_compiled_gate_state() {
    assert_eq!(HTTP_SERVER_COMPILED_IN, cfg!(feature = "http-server"));
}

/// The default build ships the HTTP transport; this is the state the
/// desktop app requires. Skipped when the slim build is under test.
#[test]
#[cfg(feature = "http-server")]
fn is_true_when_the_http_server_feature_is_on() {
    const {
        assert!(HTTP_SERVER_COMPILED_IN);
    }
}

/// The slim build must report honestly, otherwise the shell's const assert
/// would pass against a listener-less core.
#[test]
#[cfg(not(feature = "http-server"))]
fn is_false_when_the_http_server_feature_is_off() {
    const {
        assert!(!HTTP_SERVER_COMPILED_IN);
    }
}
