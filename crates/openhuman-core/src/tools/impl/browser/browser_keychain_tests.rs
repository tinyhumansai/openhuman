use super::*;
use serde_json::json;
use std::sync::Arc;
use tinycomputer_bus::browser::Target;

fn client(command: Option<&str>) -> BrowserClient {
    let mut config = crate::config::Config::default();
    config.browser.keychain_command = command.map(str::to_owned);
    BrowserClient::new(Arc::new(config))
}

fn fill(value: &str) -> Action {
    Action::Fill {
        target: Target::Ref { value: "e1".into() },
        value: value.into(),
    }
}

#[test]
fn a_placeholder_is_the_whole_value_and_names_a_site() {
    assert_eq!(
        placeholder_site("{{keychain:example.com}}").as_deref(),
        Some("example.com")
    );
    assert_eq!(
        placeholder_site(" {{keychain:Sub.Example.COM.}} ").as_deref(),
        Some("sub.example.com")
    );
    for value in [
        "hunter2",
        "{{keychain:}}",
        "{{keychain:localhost}}",
        "x{{keychain:example.com}}",
        "{{keychain:example.com}}x",
        "{{keychain:exa mple.com}}",
        "{{keychain:example.com/login}}",
    ] {
        assert_eq!(placeholder_site(value), None, "{value}");
    }
}

#[test]
fn only_an_https_page_on_the_site_or_under_it_matches() {
    assert_eq!(
        host_on_site("https://example.com/login", "example.com").as_deref(),
        Some("example.com")
    );
    assert_eq!(
        host_on_site("https://accounts.example.com/", "example.com").as_deref(),
        Some("accounts.example.com")
    );
    for url in [
        "http://example.com/",
        "https://example.com.evil.io/",
        "https://notexample.com/",
        "https://evil.io/?next=example.com",
        "about:blank",
        "not a url",
    ] {
        assert_eq!(host_on_site(url, "example.com"), None, "{url}");
    }
}

#[test]
fn the_password_is_masked_everywhere_in_a_reply() {
    let reply = json!({"value": "pw-123", "nested": ["a pw-123 b", {"pw-123": 1}], "n": 3});
    let masked = redact_value(reply, "pw-123");
    let text = masked.to_string();
    assert!(!text.contains("pw-123"), "{text}");
    assert_eq!(masked["value"], json!(MASK));
    assert_eq!(masked["n"], json!(3));
    assert_eq!(redact("anything", ""), "anything");
}

#[tokio::test]
async fn an_action_without_a_placeholder_is_left_alone() {
    let mut action = fill("Teeny");
    let got = substitute(&client(None), &SessionId::new("s-1"), &mut action)
        .await
        .unwrap();
    assert!(got.is_none());
    assert!(matches!(action, Action::Fill { value, .. } if value == "Teeny"));
    let mut click = Action::Click {
        target: Target::Ref { value: "e1".into() },
        new_tab: false,
    };
    assert!(
        substitute(&client(None), &SessionId::new("s-1"), &mut click)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_placeholder_without_a_keychain_or_in_part_of_a_value_is_refused() {
    let mut action = fill("{{keychain:example.com}}");
    let error = substitute(&client(None), &SessionId::new("s-1"), &mut action)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("no keychain is configured"),
        "{error}"
    );
    let mut partial = fill("pre{{keychain:example.com}}");
    let error = substitute(
        &client(Some("/bin/true")),
        &SessionId::new("s-1"),
        &mut partial,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("whole value"), "{error}");
}

#[cfg(unix)]
fn script(dir: &std::path::Path, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("keychain");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path.display().to_string()
}

#[cfg(unix)]
#[tokio::test]
async fn the_command_is_asked_for_the_page_host_and_answers_one_password() {
    let dir = tempfile::tempdir().unwrap();
    let command = script(
        dir.path(),
        r#"[ "$1" = "accounts.example.com" ] && printf 'pw-%s\n' 123"#,
    );
    assert_eq!(
        run_command(&command, "accounts.example.com")
            .await
            .unwrap()
            .as_str(),
        "pw-123"
    );
    let error = run_command(&command, "evil.io").await.unwrap_err();
    assert!(
        error.to_string().contains("no password for evil.io"),
        "{error}"
    );
    let two = script(dir.path(), "printf 'a\\nb\\n'");
    assert!(run_command(&two, "example.com").await.is_err());
    assert!(run_command("relative/keychain", "example.com")
        .await
        .is_err());
}

#[test]
fn a_filled_password_stays_masked_in_later_replies() {
    let secret = Zeroizing::new("later-pw-789".to_owned());
    assert_eq!(redact_filled("value: later-pw-789"), "value: later-pw-789");
    remember(&secret);
    let snapshot = r#"textbox "Name" value="later-pw-789""#;
    assert_eq!(
        redact_filled(snapshot),
        format!(r#"textbox "Name" value="{MASK}""#)
    );
    assert_eq!(redact_filled("nothing here"), "nothing here");
}
