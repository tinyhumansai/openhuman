use super::*;

#[test]
fn action_to_args_preserves_find_shape() {
    let args = action_to_args(
        BrowserAction::Find {
            by: "label".into(),
            value: "Email".into(),
            action: "fill".into(),
            fill_value: Some("a@example.com".into()),
        },
        None,
    );

    assert_eq!(args["action"], "find");
    assert_eq!(args["find_action"], "fill");
    assert_eq!(args["fill_value"], "a@example.com");
}
