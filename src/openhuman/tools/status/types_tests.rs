use super::*;

#[test]
fn recoverable_iff_recoverable_category() {
    assert!(FailureCategory::Recoverable.is_recoverable());
    assert!(!FailureCategory::BlockedByPolicy.is_recoverable());
    assert!(!FailureCategory::NeedsUserConfirmation.is_recoverable());
    assert!(!FailureCategory::UserDeclined.is_recoverable());
}

#[test]
fn every_class_maps_to_expected_category() {
    use FailureCategory::*;
    use ToolFailureClass::*;
    assert_eq!(ServiceUnavailable.category(), Recoverable);
    assert_eq!(ModelConnection.category(), Recoverable);
    assert_eq!(Timeout.category(), Recoverable);
    assert_eq!(Unknown.category(), Recoverable);
    assert_eq!(
        ToolFailureClass::BlockedByPolicy.category(),
        FailureCategory::BlockedByPolicy
    );
    assert_eq!(MissingPermission.category(), NeedsUserConfirmation);
    assert_eq!(MissingApp.category(), NeedsUserConfirmation);
    assert_eq!(BadCredentials.category(), NeedsUserConfirmation);
    assert_eq!(Denied.category(), UserDeclined);
    assert_eq!(ApprovalExpired.category(), UserDeclined);
}

#[test]
fn types_round_trip_through_json() {
    let f = ClassifiedFailure {
        class: ToolFailureClass::Timeout,
        category: FailureCategory::Recoverable,
        cause_plain: "took too long".into(),
        next_action: "try again".into(),
        recoverable: true,
    };
    let json = serde_json::to_string(&f).unwrap();
    let back: ClassifiedFailure = serde_json::from_str(&json).unwrap();
    assert_eq!(f, back);
}

#[test]
fn lifecycle_state_serializes_to_variant_name() {
    let json = serde_json::to_string(&ToolLifecycleState::NeedsUserInput).unwrap();
    assert_eq!(json, "\"NeedsUserInput\"");
}
