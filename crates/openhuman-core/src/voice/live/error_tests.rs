use super::*;

#[test]
fn maps_library_errors_to_stable_codes() {
    let cases = [
        (LiveError::Unauthorized, "unauthorized"),
        (LiveError::InsufficientCredits, "insufficient_credits"),
        (LiveError::RateLimited, "rate_limited"),
        (LiveError::Timeout, "timeout"),
        (LiveError::InvalidConfig("x".into()), "invalid_request"),
        (LiveError::Connect("x".into()), "connect"),
        (LiveError::Provider("x".into()), "provider"),
        (LiveError::Closed, "provider"),
    ];
    for (error, code) in cases {
        assert_eq!(LiveVoiceError::from(error).code, code);
    }
}

#[test]
fn constructors_and_display() {
    assert_eq!(LiveVoiceError::not_configured("m").code, "not_configured");
    assert_eq!(LiveVoiceError::invalid("m").code, "invalid_request");
    assert_eq!(LiveVoiceError::backend("m").code, "backend");
    assert_eq!(LiveVoiceError::internal("m").code, "internal");
    assert_eq!(LiveVoiceError::new("x", "y").to_string(), "x: y");
}
