use super::*;

#[test]
fn normalize_cmdorctrl_macos() {
    let result = normalize_hotkey_for_rdev("CmdOrCtrl+Shift+D");
    if cfg!(target_os = "macos") {
        assert_eq!(result, "cmd+shift+d");
    } else {
        assert_eq!(result, "ctrl+shift+d");
    }
}

#[test]
fn normalize_plain_keys() {
    assert_eq!(normalize_hotkey_for_rdev("Ctrl+Space"), "ctrl+space");
}

#[test]
fn normalize_preserves_structure() {
    assert_eq!(normalize_hotkey_for_rdev("Alt+Shift+F5"), "alt+shift+f5");
}

#[test]
fn subscribe_returns_receiver() {
    let _rx = subscribe_dictation_events();
}

#[test]
fn publish_dictation_event_reaches_subscriber() {
    let mut rx = subscribe_dictation_events();
    publish_dictation_event(DictationEvent {
        event_type: "pressed".to_string(),
        hotkey: "chat_button".to_string(),
        activation_mode: "toggle".to_string(),
    });
    let evt = rx.try_recv().expect("should receive dictation event");
    assert_eq!(evt.event_type, "pressed");
    assert_eq!(evt.hotkey, "chat_button");
}

#[test]
fn publish_transcription_reaches_subscriber() {
    let mut rx = subscribe_transcription_results();
    publish_transcription("hello world".to_string());
    let text = rx.try_recv().expect("should receive transcription");
    assert_eq!(text, "hello world");
}

#[test]
fn normalize_commandorcontrol_alias() {
    let result = normalize_hotkey_for_rdev("CommandOrControl+Alt+K");
    if cfg!(target_os = "macos") {
        assert_eq!(result, "cmd+alt+k");
    } else {
        assert_eq!(result, "ctrl+alt+k");
    }
}

#[test]
fn dictation_event_serializes_wire_type_field() {
    let evt = DictationEvent {
        event_type: "released".to_string(),
        hotkey: "fn".to_string(),
        activation_mode: "push".to_string(),
    };
    let json = serde_json::to_value(evt).expect("serialize dictation event");
    assert_eq!(json["type"], "released");
    assert_eq!(json["hotkey"], "fn");
    assert_eq!(json["activation_mode"], "push");
}

#[tokio::test]
async fn start_if_enabled_returns_early_when_config_disabled() {
    // Fast path — `enabled=false` → the fn returns without spawning.
    let mut config = Config::default();
    config.dictation.enabled = false;
    start_if_enabled(&config).await;
    // No panic = pass. The absence of a spawned hotkey task is what
    // we're verifying; hard to assert directly without internals.
}

#[tokio::test]
async fn start_if_enabled_returns_early_when_hotkey_empty() {
    let mut config = Config::default();
    config.dictation.enabled = true;
    config.dictation.hotkey = String::new();
    start_if_enabled(&config).await;
}

#[tokio::test]
async fn start_if_enabled_returns_early_when_hotkey_unparseable() {
    let mut config = Config::default();
    config.dictation.enabled = true;
    config.dictation.hotkey = "not a real hotkey".into();
    start_if_enabled(&config).await;
}

// On macOS the rdev listener must never start — TSMGetInputSourceProperty
// must run on the main dispatch queue; rdev fires its callback on a
// background thread and crashes with EXC_BREAKPOINT on macOS 26. (#2677)
#[cfg(target_os = "macos")]
#[tokio::test]
async fn start_if_enabled_is_noop_on_macos_with_valid_hotkey() {
    let mut config = Config::default();
    config.dictation.enabled = true;
    config.dictation.hotkey = "ctrl+space".into();
    // Must return without panicking or spawning the rdev listener.
    start_if_enabled(&config).await;
    // No rdev thread was started: the global handle remains None.
    let guard = LISTENER_HANDLE.lock().expect("lock");
    assert!(
        guard.is_none(),
        "rdev listener must not be started on macOS"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn start_if_enabled_is_noop_on_macos_with_fn_hotkey() {
    let mut config = Config::default();
    config.dictation.enabled = true;
    config.dictation.hotkey = "fn".into();
    start_if_enabled(&config).await;
    let guard = LISTENER_HANDLE.lock().expect("lock");
    assert!(
        guard.is_none(),
        "rdev listener must not be started on macOS"
    );
}

#[test]
fn normalize_maps_shift_and_alt_verbatim() {
    let result = normalize_hotkey_for_rdev("Shift+Alt+D");
    assert_eq!(result, "shift+alt+d");
}

#[test]
fn normalize_handles_lowercase_input() {
    assert_eq!(normalize_hotkey_for_rdev("cmd+d"), "cmd+d");
}

#[test]
fn normalize_preserves_function_keys() {
    assert_eq!(normalize_hotkey_for_rdev("F12"), "f12");
}

#[test]
fn normalize_trims_whitespace_between_segments() {
    let result = normalize_hotkey_for_rdev("  cmd  + shift  +  d  ");
    assert_eq!(result, "cmd+shift+d");
}
