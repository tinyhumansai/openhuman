use super::*;

#[test]
fn every_variant_has_its_stable_code() {
    assert_eq!(MemoryError::Off("x".into()).code(), MEMORY_OFF);
    assert_eq!(MemoryError::Unsupported("x".into()).code(), UNSUPPORTED);
    assert_eq!(MemoryError::invalid("x").code(), INVALID_REQUEST);
    assert_eq!(MemoryError::Unauthorized("x".into()).code(), UNAUTHORIZED);
    assert_eq!(MemoryError::Engine("x".into()).code(), ENGINE);
}

#[test]
fn structured_error_carries_the_code_and_flags_user_states() {
    let off = MemoryError::Off("sign in".into()).to_structured();
    assert_eq!(off.data.as_ref().unwrap()["code"], MEMORY_OFF);
    assert_eq!(off.data.as_ref().unwrap()["kind"], MEMORY_OFF);
    assert!(off.expected_user_state);
    assert!(off.message.contains("sign in"));

    let engine = MemoryError::Engine("boom".into()).to_structured();
    assert!(!engine.expected_user_state);
}

#[test]
fn tinymemory_errors_map_onto_the_taxonomy() {
    use tinymemory::Error as E;
    let cases = [
        (E::Unsupported("a".into()), UNSUPPORTED),
        (E::InvalidRequest("a".into()), INVALID_REQUEST),
        (E::NotFound("a".into()), INVALID_REQUEST),
        (E::Config("a".into()), INVALID_REQUEST),
        (E::Unauthorized("a".into()), UNAUTHORIZED),
        (E::Conflict("a".into()), ENGINE),
        (E::Unavailable("a".into()), ENGINE),
        (E::Engine("a".into()), ENGINE),
    ];
    for (error, code) in cases {
        assert_eq!(MemoryError::from(error).code(), code);
    }
}

#[test]
fn into_string_encodes_the_structured_error() {
    let encoded: String = MemoryError::invalid("bad").into();
    assert!(encoded.contains(INVALID_REQUEST), "{encoded}");
    assert!(encoded.contains("bad"), "{encoded}");
}
