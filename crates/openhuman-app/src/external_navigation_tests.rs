use super::*;

fn url(s: &str) -> Url {
    Url::parse(s).unwrap()
}

#[test]
fn remote_pages_are_cancelled_and_handed_to_the_os_browser() {
    for href in [
        "https://github.com/tinyhumansai/openhuman",
        "http://example.com/login?token=abc",
        // Loopback that is not the dev server is still a remote page to the webview.
        "http://localhost:8000/",
        "https://tauri.localhost.example.com/",
        // Same host, different origin: a loopback service on another port.
        "http://tauri.localhost:8000/",
        // Same host, other scheme than the app is configured with (`useHttpsScheme` unset).
        "https://tauri.localhost/",
    ] {
        assert_eq!(
            navigation_handoff("main", &url(href), "http", None),
            Some(url(href)),
            "{href} must not load in the main webview"
        );
    }
}

#[test]
fn app_origins_stay_in_the_main_webview() {
    for href in [
        "tauri://localhost/#/chat",
        "http://tauri.localhost/#/chat",
        "about:blank",
        "data:text/html,<p>hi</p>",
        "blob:tauri://localhost/5e1c",
    ] {
        assert_eq!(
            navigation_handoff("main", &url(href), "http", None),
            None,
            "{href}"
        );
    }
}

#[test]
fn windows_app_origin_follows_the_configured_scheme() {
    assert_eq!(
        navigation_handoff(
            "main",
            &url("https://tauri.localhost/#/chat"),
            "https",
            None
        ),
        None
    );
    assert!(
        navigation_handoff("main", &url("http://tauri.localhost/#/chat"), "https", None).is_some()
    );
}

#[test]
fn dev_server_is_an_app_origin_only_when_it_is_the_dev_url() {
    let dev = url("http://localhost:1420");
    assert_eq!(
        navigation_handoff(
            "main",
            &url("http://localhost:1420/#/chat"),
            "http",
            Some(&dev)
        ),
        None
    );
    assert!(
        navigation_handoff("main", &url("http://localhost:1420/#/chat"), "http", None).is_some()
    );
}

#[test]
fn other_webviews_are_left_to_their_own_handlers() {
    assert_eq!(
        navigation_handoff("ptt-overlay", &url("https://github.com/"), "http", None),
        None
    );
}

#[test]
fn widget_sandbox_origin_stays_in_the_main_webview() {
    assert_eq!(
        navigation_handoff(
            "main",
            &url("http://ohwidget.localhost/proxy"),
            "http",
            None
        ),
        None
    );
    assert_eq!(
        navigation_handoff("main", &url("ohwidget://localhost/proxy"), "http", None),
        None
    );
    let other_port = url("http://ohwidget.localhost:8080/proxy");
    assert_eq!(
        navigation_handoff("main", &other_port, "http", None),
        Some(other_port)
    );
}
