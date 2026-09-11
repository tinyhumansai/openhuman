use super::*;

#[test]
fn runtime_requirement_parses_aliases() {
    assert_eq!(
        RuntimeRequirement::from_optional(None).unwrap(),
        RuntimeRequirement::All
    );
    assert_eq!(
        RuntimeRequirement::from_optional(Some("nodejs")).unwrap(),
        RuntimeRequirement::Node
    );
    assert_eq!(
        RuntimeRequirement::from_optional(Some("python3")).unwrap(),
        RuntimeRequirement::Python
    );
    assert!(RuntimeRequirement::from_optional(Some("ruby")).is_err());
}
