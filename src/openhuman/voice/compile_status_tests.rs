use super::VOICE_COMPILED_IN;

/// Pins the constant to the gate rather than to a hardcoded value: the
/// assertion inverts with the feature, so it holds for both the default
/// build and the slim (`--no-default-features`) build.
#[test]
fn reports_the_compiled_gate_state() {
    assert_eq!(VOICE_COMPILED_IN, cfg!(feature = "voice"));
}

/// The default build ships voice; this is the state the desktop app
/// requires (#4901). Skipped when the slim build is under test.
#[test]
#[cfg(feature = "voice")]
fn is_true_when_the_voice_feature_is_on() {
    assert!(VOICE_COMPILED_IN);
}

/// The slim build must report honestly, otherwise the shell's const assert
/// would pass against a stubbed core and #4901 could ship again.
#[test]
#[cfg(not(feature = "voice"))]
fn is_false_when_the_voice_feature_is_off() {
    assert!(!VOICE_COMPILED_IN);
}
