use super::*;
use std::sync::atomic::AtomicBool;

fn combo() -> HotkeyCombination {
    parse_hotkey("ctrl+space").expect("test hotkey")
}

#[test]
fn parse_simple_hotkey() {
    let combo = parse_hotkey("ctrl+shift+space").unwrap();
    assert_eq!(combo.trigger, Key::Space);
    assert!(combo.modifiers.contains(&Key::ControlLeft));
    assert!(combo.modifiers.contains(&Key::ShiftLeft));
}

#[test]
fn parse_single_key() {
    let combo = parse_hotkey("f5").unwrap();
    assert_eq!(combo.trigger, Key::F5);
    assert!(combo.modifiers.is_empty());
}

#[test]
fn parse_cmd_key() {
    let combo = parse_hotkey("cmd+space").unwrap();
    assert_eq!(combo.trigger, Key::Space);
    assert!(combo.modifiers.contains(&Key::MetaLeft));
}

#[test]
fn parse_function_key() {
    let combo = parse_hotkey("fn").unwrap();
    assert_eq!(combo.trigger, Key::Function);
    assert!(combo.modifiers.is_empty());
}

#[test]
fn parse_empty_errors() {
    assert!(parse_hotkey("").is_err());
}

#[test]
fn parse_unknown_key_errors() {
    assert!(parse_hotkey("ctrl+unknownkey").is_err());
}

#[test]
fn activation_mode_default_is_push() {
    assert_eq!(ActivationMode::default(), ActivationMode::Push);
}

#[test]
fn parse_hotkey_trims_and_ignores_empty_segments() {
    let combo = parse_hotkey("  ctrl +  + shift + space ").unwrap();
    assert_eq!(combo.trigger, Key::Space);
    assert!(combo.modifiers.contains(&Key::ControlLeft));
    assert!(combo.modifiers.contains(&Key::ShiftLeft));
    assert_eq!(combo.modifiers.len(), 2);
}

#[test]
fn parse_hotkey_supports_aliases_and_right_side_modifiers() {
    let combo = parse_hotkey("rctrl+rshift+return").unwrap();
    assert_eq!(combo.trigger, Key::Return);
    assert!(combo.modifiers.contains(&Key::ControlRight));
    assert!(combo.modifiers.contains(&Key::ShiftRight));
}

#[test]
fn parse_hotkey_rejects_whitespace_only() {
    let err = parse_hotkey("   ").expect_err("whitespace-only hotkey should fail");
    assert!(err.contains("empty"));
}

#[test]
fn process_hotkey_event_push_requires_modifier_then_releases() {
    let combo = combo();
    let is_active = AtomicBool::new(false);
    let mut pressed = HashSet::new();

    let no_emit = process_hotkey_event(
        EventType::KeyPress(Key::Space),
        &combo,
        ActivationMode::Push,
        &mut pressed,
        &is_active,
    );
    assert!(no_emit.is_empty());

    process_hotkey_event(
        EventType::KeyPress(Key::ControlLeft),
        &combo,
        ActivationMode::Push,
        &mut pressed,
        &is_active,
    );
    let pressed_event = process_hotkey_event(
        EventType::KeyPress(Key::Space),
        &combo,
        ActivationMode::Push,
        &mut pressed,
        &is_active,
    );
    assert_eq!(pressed_event, vec![HotkeyEvent::Pressed]);

    let release_event = process_hotkey_event(
        EventType::KeyRelease(Key::Space),
        &combo,
        ActivationMode::Push,
        &mut pressed,
        &is_active,
    );
    assert_eq!(release_event, vec![HotkeyEvent::Released]);
}

#[test]
fn process_hotkey_event_push_second_press_is_release_fallback() {
    let combo = combo();
    let is_active = AtomicBool::new(false);
    let mut pressed = HashSet::from([Key::ControlLeft]);

    let first = process_hotkey_event(
        EventType::KeyPress(Key::Space),
        &combo,
        ActivationMode::Push,
        &mut pressed,
        &is_active,
    );
    let second = process_hotkey_event(
        EventType::KeyPress(Key::Space),
        &combo,
        ActivationMode::Push,
        &mut pressed,
        &is_active,
    );

    assert_eq!(first, vec![HotkeyEvent::Pressed]);
    assert_eq!(second, vec![HotkeyEvent::Released]);
}

#[test]
fn process_hotkey_event_tap_toggles_on_each_press() {
    let combo = combo();
    let is_active = AtomicBool::new(false);
    let mut pressed = HashSet::from([Key::ControlLeft]);

    let first = process_hotkey_event(
        EventType::KeyPress(Key::Space),
        &combo,
        ActivationMode::Tap,
        &mut pressed,
        &is_active,
    );
    let second = process_hotkey_event(
        EventType::KeyPress(Key::Space),
        &combo,
        ActivationMode::Tap,
        &mut pressed,
        &is_active,
    );

    assert_eq!(first, vec![HotkeyEvent::Pressed]);
    assert_eq!(second, vec![HotkeyEvent::Released]);
}
