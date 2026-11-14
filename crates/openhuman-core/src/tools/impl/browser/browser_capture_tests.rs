use super::*;
use std::sync::Arc;

fn client_with_dir(dir: Option<&str>) -> BrowserClient {
    let mut config = crate::config::Config::default();
    config.browser.screenshot_dir = dir.map(str::to_owned);
    BrowserClient::new(Arc::new(config))
}

#[test]
fn a_picture_follows_actions_that_change_the_page_never_typing() {
    for kind in ["click", "double_click", "press", "select", "check"] {
        assert!(follows(kind), "{kind}");
    }
    for kind in ["fill", "type", "hover", "scroll", "get_text", "is_visible", "wait_for"] {
        assert!(!follows(kind), "{kind}");
    }
}

#[test]
fn pictures_go_only_to_an_absolute_configured_folder() {
    assert_eq!(folder(&client_with_dir(None)), None);
    assert_eq!(folder(&client_with_dir(Some("  "))), None);
    assert_eq!(folder(&client_with_dir(Some("state/shots"))), None);
    assert_eq!(
        folder(&client_with_dir(Some("/srv/state/shots"))),
        Some(PathBuf::from("/srv/state/shots"))
    );
}

#[test]
fn a_page_showing_a_keychain_password_is_not_captured() {
    let secret = zeroize::Zeroizing::new("capture-test-pw-7d1c9e".to_owned());
    super::super::keychain::remember(&secret);
    assert!(super::super::keychain::any_filled());
    assert!(shows_filled_password(&[
        "textbox \"Password\": capture-test-pw-7d1c9e"
    ]));
    assert!(shows_filled_password(&["ok", "your password is capture-test-pw-7d1c9e"]));
    assert!(!shows_filled_password(&[
        "textbox \"Password\" [ref=e4]",
        "Welcome back, Teeny"
    ]));
}

#[test]
fn saved_pictures_sort_in_the_order_taken_and_are_whole() {
    let dir = tempfile::tempdir().unwrap();
    let first = save(dir.path(), b"\x89PNGone", "png").unwrap();
    let second = save(dir.path(), b"\x89PNGtwo", "png").unwrap();
    assert!(first.file_name() < second.file_name());
    assert_eq!(std::fs::read(&second).unwrap(), b"\x89PNGtwo");
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|n| !n.ends_with(".part")), "{names:?}");
    assert_eq!(file_name(1_700_000_000_000, 7, "jpeg"), "1700000000000-000007.jpeg");
}

#[test]
fn the_folder_keeps_only_the_newest_pictures() {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..(MAX_KEPT + 5) {
        std::fs::write(dir.path().join(file_name(1_000 + n as u128, 0, "png")), b"x").unwrap();
    }
    std::fs::write(dir.path().join("notes.txt"), b"kept").unwrap();
    prune(dir.path(), MAX_KEPT);
    let mut pictures: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".png"))
        .collect();
    pictures.sort();
    assert_eq!(pictures.len(), MAX_KEPT);
    assert_eq!(pictures[0], file_name(1_005, 0, "png"));
    assert!(dir.path().join("notes.txt").exists());
}
