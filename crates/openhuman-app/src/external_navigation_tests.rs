use super::*;

#[test]
fn remote_navigation_is_external_but_app_origins_are_preserved() {
    let dev = Url::parse("http://localhost:1420").unwrap();
    for href in [
        "https://github.com/example/repo/pull/1",
        "http://localhost:8000",
        "https://tauri.localhost.example.com",
    ] {
        assert!(
            is_external(&Url::parse(href).unwrap(), Some(&dev)),
            "{href}"
        );
    }
    for href in [
        "tauri://localhost/#/chat",
        "http://tauri.localhost/#/chat",
        "https://tauri.localhost/#/chat",
        "http://localhost:1420/#/chat",
    ] {
        assert!(
            !is_external(&Url::parse(href).unwrap(), Some(&dev)),
            "{href}"
        );
    }
    assert!(is_external(&dev, None));
}

#[test]
fn internal_routes_and_child_webviews_do_not_invoke_the_os_opener() {
    for (label, href) in [
        ("main", "tauri://localhost/#/chat"),
        ("preview", "https://example.com/preview"),
    ] {
        assert!(handle_navigation(
            label,
            &Url::parse(href).unwrap(),
            None,
            |_| { panic!("an internal route or child webview must not be handed to the OS") }
        ));
    }
}

#[tokio::test]
async fn external_navigation_is_cancelled_even_when_the_os_opener_fails() {
    for opens_successfully in [true, false] {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let url = Url::parse("https://example.com/repo/pull/1").unwrap();
        assert!(!handle_navigation("main", &url, None, move |target| {
            tx.send(target).unwrap();
            opens_successfully
        }));
        assert_eq!(rx.await.unwrap(), url.as_str());
    }
}
