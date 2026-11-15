use super::*;

fn left(url: &str) -> crate::modules::browser::BrowserCallError {
    crate::modules::browser::BrowserCallError::Bus {
        name: "BlockedByPolicy".into(),
        message: format!(
            "the page reached, {url}, is not permitted by this session's allowed origins; the session left it"
        ),
    }
}

#[test]
fn a_click_that_left_for_another_site_names_the_page() {
    assert_eq!(
        left_for(&left("https://www.spacex.com/launches/starship-flight-15")).as_deref(),
        Some("https://www.spacex.com/launches/starship-flight-15")
    );
    // A URL with a comma in it is still read whole.
    assert_eq!(
        left_for(&left("https://example.com/a,b?c=1,2")).as_deref(),
        Some("https://example.com/a,b?c=1,2")
    );
}

#[test]
fn other_failures_are_not_a_page_left_for_another_site() {
    use crate::modules::browser::BrowserCallError;
    for error in [
        BrowserCallError::Bus {
            name: "BlockedByPolicy".into(),
            message: "navigation to https://x.example/ is not permitted by this session's allowed origins".into(),
        },
        BrowserCallError::Bus {
            name: "StaleRef".into(),
            message: "the page reached, https://a.example/, is not permitted by this session's allowed origins; the session left it".into(),
        },
        BrowserCallError::Policy("local/private host '127.0.0.1' is blocked".into()),
        // Not a public https page: never offered as a place to open.
        left("http://example.com/"),
        left("https://127.0.0.1/admin"),
        left("not a url"),
    ] {
        assert_eq!(left_for(&error), None, "{error}");
    }
}

#[test]
fn the_reply_tells_the_agent_to_open_the_page_itself() {
    let reply = left_reply(
        "click",
        "https://www.spacex.com/launches/starship-flight-15",
    );
    assert_eq!(reply["ran"], serde_json::json!(true));
    assert_eq!(
        reply["left_for_another_site"],
        serde_json::json!("https://www.spacex.com/launches/starship-flight-15")
    );
    let next = reply["next"].as_str().unwrap();
    assert!(next.contains("action=open"), "{next}");
    assert!(
        next.contains("https://www.spacex.com/launches/starship-flight-15"),
        "{next}"
    );
}
