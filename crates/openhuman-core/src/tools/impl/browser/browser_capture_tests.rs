use super::*;

#[test]
fn a_picture_follows_actions_that_change_the_page_never_typing() {
    for kind in ["click", "double_click", "press", "select", "check"] {
        assert!(follows(kind), "{kind}");
    }
    for kind in [
        "fill",
        "type",
        "hover",
        "scroll",
        "get_text",
        "is_visible",
        "wait_for",
    ] {
        assert!(!follows(kind), "{kind}");
    }
}

#[test]
fn a_page_showing_a_keychain_password_is_not_captured() {
    let secret = zeroize::Zeroizing::new("capture-test-pw-7d1c9e".to_owned());
    super::super::keychain::remember(&secret);
    assert!(super::super::keychain::any_filled());
    assert!(shows_filled_password(&[
        "textbox \"Password\": capture-test-pw-7d1c9e"
    ]));
    assert!(shows_filled_password(&[
        "ok",
        "your password is capture-test-pw-7d1c9e"
    ]));
    assert!(!shows_filled_password(&[
        "textbox \"Password\" [ref=e4]",
        "Welcome back, Teeny"
    ]));
}

#[tokio::test]
async fn no_sink_means_no_picture_and_no_browser_call() {
    // With nothing registered the capture returns before it reaches the
    // module (which this test has none of): no error, nothing taken.
    let client = BrowserClient::new(std::sync::Arc::new(crate::config::Config::default()));
    capture(&client, &SessionId::new("s-1"), "open").await;
}

#[test]
fn the_page_check_reads_only_a_list_of_strings() {
    assert_eq!(
        page_texts(&serde_json::json!(["Welcome", "teeny@agentmail.to"])),
        Some(vec!["Welcome".to_owned(), "teeny@agentmail.to".to_owned()])
    );
    // Anything else is a page that could not be checked: no picture.
    assert_eq!(page_texts(&serde_json::json!({"x": 1})), None);
    assert_eq!(page_texts(&serde_json::json!(["ok", 3])), None);
    assert_eq!(page_texts(&serde_json::Value::Null), None);
    // The script leaves a masked password field's own value out.
    assert!(PAGE_TEXT_JS.contains("el.type === 'password'"));
    // An unreadable frame makes the whole answer null (not checked), and a
    // shadow root is recognised in any frame's realm.
    assert!(PAGE_TEXT_JS.contains("return unreadable ? null : out;"));
    assert!(!PAGE_TEXT_JS.contains("instanceof ShadowRoot"));
}
