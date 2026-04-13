//! Global hotkey listener using rdev.
//!
//! Monitors keyboard events system-wide and fires callbacks when a
//! configurable key combination is pressed/released. Supports two
//! activation modes: **tap** (toggle on press) and **push** (hold to
//! record, release to stop).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use log::{debug, error, info};
use parking_lot::Mutex;
use rdev::{listen, Event, EventType, Key};
use tokio::sync::mpsc;

const LOG_PREFIX: &str = "[voice_hotkey]";

/// Activation mode for the voice hotkey.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ActivationMode {
    /// Single press toggles recording on/off.
    Tap,
    /// Hold to record, release to stop.
    #[default]
    Push,
}

/// Events emitted by the hotkey listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// The hotkey was pressed (start recording).
    Pressed,
    /// The hotkey was released (stop recording — only relevant in Push mode).
    Released,
}

/// Parsed hotkey combination (e.g. Ctrl+Shift+Space).
#[derive(Debug, Clone)]
pub struct HotkeyCombination {
    /// Modifier keys that must be held.
    pub modifiers: HashSet<Key>,
    /// The primary trigger key.
    pub trigger: Key,
}

/// Handle to a running hotkey listener. Drop to stop.
pub struct HotkeyListenerHandle {
    stop_flag: Arc<AtomicBool>,
    _thread: Option<std::thread::JoinHandle<()>>,
}

impl HotkeyListenerHandle {
    /// Signal the listener to ignore further events.
    ///
    /// Note: this does **not** terminate the listener thread. `rdev::listen`
    /// blocks in the platform event loop and provides no cancellation API
    /// (rdev 0.5). The thread stays alive until the process exits; the
    /// stop flag merely causes the callback to discard all events.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        info!("{LOG_PREFIX} hotkey listener signaled to skip events");
    }
}

impl Drop for HotkeyListenerHandle {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }
}

fn process_hotkey_event(
    event_type: EventType,
    hotkey: &HotkeyCombination,
    mode: ActivationMode,
    pressed_keys: &mut HashSet<Key>,
    is_active: &AtomicBool,
) -> Vec<HotkeyEvent> {
    let mut emitted = Vec::new();

    match event_type {
        EventType::KeyPress(key) => {
            let is_trigger = key == hotkey.trigger;
            pressed_keys.insert(key);

            if !is_trigger {
                return emitted;
            }

            if !hotkey.modifiers.iter().all(|m| pressed_keys.contains(m)) {
                return emitted;
            }

            let was_active = is_active.load(Ordering::SeqCst);
            debug!(
                "{LOG_PREFIX} KeyPress trigger={:?} was_active={was_active} mode={mode:?}",
                key
            );

            match mode {
                ActivationMode::Tap => {
                    if was_active {
                        is_active.store(false, Ordering::SeqCst);
                        info!("{LOG_PREFIX} tap → Released");
                        emitted.push(HotkeyEvent::Released);
                    } else {
                        is_active.store(true, Ordering::SeqCst);
                        info!("{LOG_PREFIX} tap → Pressed");
                        emitted.push(HotkeyEvent::Pressed);
                    }
                }
                ActivationMode::Push => {
                    if !was_active {
                        is_active.store(true, Ordering::SeqCst);
                        info!("{LOG_PREFIX} push → Pressed");
                        emitted.push(HotkeyEvent::Pressed);
                    } else {
                        is_active.store(false, Ordering::SeqCst);
                        info!("{LOG_PREFIX} push → Released (fallback, missed KeyRelease)");
                        emitted.push(HotkeyEvent::Released);
                    }
                }
            }
        }
        EventType::KeyRelease(key) => {
            pressed_keys.remove(&key);

            if key != hotkey.trigger {
                return emitted;
            }

            debug!(
                "{LOG_PREFIX} KeyRelease trigger={:?} is_active={}",
                key,
                is_active.load(Ordering::SeqCst)
            );

            if mode == ActivationMode::Push && is_active.swap(false, Ordering::SeqCst) {
                info!("{LOG_PREFIX} push → Released");
                emitted.push(HotkeyEvent::Released);
            }
        }
        _ => {}
    }

    emitted
}

/// Parse a hotkey string like "ctrl+shift+space" or "fn" into a `HotkeyCombination`.
pub fn parse_hotkey(hotkey_str: &str) -> Result<HotkeyCombination, String> {
    let parts: Vec<&str> = hotkey_str
        .split('+')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    if parts.is_empty() {
        return Err("hotkey string is empty".to_string());
    }

    let mut modifiers = HashSet::new();
    let mut trigger = None;

    for (i, part) in parts.iter().enumerate() {
        let key = string_to_key(part)?;
        if i < parts.len() - 1 {
            modifiers.insert(key);
        } else {
            trigger = Some(key);
        }
    }

    let trigger = trigger.ok_or_else(|| "no trigger key specified".to_string())?;

    debug!(
        "{LOG_PREFIX} parsed hotkey: modifiers={:?} trigger={:?}",
        modifiers, trigger
    );

    Ok(HotkeyCombination { modifiers, trigger })
}

/// Start the global hotkey listener.
///
/// Returns a handle (drop to stop) and a receiver for hotkey events.
/// The listener runs on a dedicated OS thread since rdev::listen is blocking.
pub fn start_listener(
    hotkey: HotkeyCombination,
    mode: ActivationMode,
) -> Result<(HotkeyListenerHandle, mpsc::UnboundedReceiver<HotkeyEvent>), String> {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::unbounded_channel();

    let stop_flag_clone = stop_flag.clone();
    let pressed_keys: Arc<Mutex<HashSet<Key>>> = Arc::new(Mutex::new(HashSet::new()));
    let is_active = Arc::new(AtomicBool::new(false));

    info!(
        "{LOG_PREFIX} starting hotkey listener, mode={mode:?}, trigger={:?}, modifiers={:?}",
        hotkey.trigger, hotkey.modifiers
    );

    let thread = std::thread::Builder::new()
        .name("voice-hotkey".into())
        .spawn(move || {
            let callback = move |event: Event| {
                if stop_flag_clone.load(Ordering::SeqCst) {
                    return;
                }
                let emitted = {
                    let mut keys = pressed_keys.lock();
                    process_hotkey_event(event.event_type, &hotkey, mode, &mut keys, &is_active)
                };
                for event in emitted {
                    let _ = tx.send(event);
                }
            };

            if let Err(e) = listen(callback) {
                error!("{LOG_PREFIX} rdev listen error: {e:?}");
            }
        })
        .map_err(|e| format!("failed to spawn hotkey listener thread: {e}"))?;

    Ok((
        HotkeyListenerHandle {
            stop_flag,
            _thread: Some(thread),
        },
        rx,
    ))
}

/// Convert a string key name to an rdev Key.
fn string_to_key(s: &str) -> Result<Key, String> {
    match s.to_lowercase().as_str() {
        // Modifiers
        "ctrl" | "control" | "leftcontrol" => Ok(Key::ControlLeft),
        "rctrl" | "rightcontrol" => Ok(Key::ControlRight),
        "shift" | "leftshift" => Ok(Key::ShiftLeft),
        "rshift" | "rightshift" => Ok(Key::ShiftRight),
        "alt" | "option" | "leftalt" => Ok(Key::Alt),
        "ralt" | "rightaltoption" => Ok(Key::AltGr),
        "meta" | "super" | "cmd" | "command" | "leftmeta" => Ok(Key::MetaLeft),
        "rmeta" | "rsuper" | "rcmd" | "rightmeta" => Ok(Key::MetaRight),

        // Common keys
        "space" => Ok(Key::Space),
        "enter" | "return" => Ok(Key::Return),
        "tab" => Ok(Key::Tab),
        "escape" | "esc" => Ok(Key::Escape),
        "backspace" => Ok(Key::Backspace),
        "delete" | "del" => Ok(Key::Delete),
        "capslock" => Ok(Key::CapsLock),
        "fn" | "function" => Ok(Key::Function),

        // F-keys
        "f1" => Ok(Key::F1),
        "f2" => Ok(Key::F2),
        "f3" => Ok(Key::F3),
        "f4" => Ok(Key::F4),
        "f5" => Ok(Key::F5),
        "f6" => Ok(Key::F6),
        "f7" => Ok(Key::F7),
        "f8" => Ok(Key::F8),
        "f9" => Ok(Key::F9),
        "f10" => Ok(Key::F10),
        "f11" => Ok(Key::F11),
        "f12" => Ok(Key::F12),

        // Navigation
        "up" | "uparrow" => Ok(Key::UpArrow),
        "down" | "downarrow" => Ok(Key::DownArrow),
        "left" | "leftarrow" => Ok(Key::LeftArrow),
        "right" | "rightarrow" => Ok(Key::RightArrow),
        "home" => Ok(Key::Home),
        "end" => Ok(Key::End),
        "pageup" | "pgup" => Ok(Key::PageUp),
        "pagedown" | "pgdn" => Ok(Key::PageDown),
        "insert" | "ins" => Ok(Key::Insert),

        // Letters
        "a" => Ok(Key::KeyA),
        "b" => Ok(Key::KeyB),
        "c" => Ok(Key::KeyC),
        "d" => Ok(Key::KeyD),
        "e" => Ok(Key::KeyE),
        "f" => Ok(Key::KeyF),
        "g" => Ok(Key::KeyG),
        "h" => Ok(Key::KeyH),
        "i" => Ok(Key::KeyI),
        "j" => Ok(Key::KeyJ),
        "k" => Ok(Key::KeyK),
        "l" => Ok(Key::KeyL),
        "m" => Ok(Key::KeyM),
        "n" => Ok(Key::KeyN),
        "o" => Ok(Key::KeyO),
        "p" => Ok(Key::KeyP),
        "q" => Ok(Key::KeyQ),
        "r" => Ok(Key::KeyR),
        "s" => Ok(Key::KeyS),
        "t" => Ok(Key::KeyT),
        "u" => Ok(Key::KeyU),
        "v" => Ok(Key::KeyV),
        "w" => Ok(Key::KeyW),
        "x" => Ok(Key::KeyX),
        "y" => Ok(Key::KeyY),
        "z" => Ok(Key::KeyZ),

        // Numbers
        "0" => Ok(Key::Num0),
        "1" => Ok(Key::Num1),
        "2" => Ok(Key::Num2),
        "3" => Ok(Key::Num3),
        "4" => Ok(Key::Num4),
        "5" => Ok(Key::Num5),
        "6" => Ok(Key::Num6),
        "7" => Ok(Key::Num7),
        "8" => Ok(Key::Num8),
        "9" => Ok(Key::Num9),

        other => Err(format!("unknown key: '{other}'")),
    }
}

#[cfg(test)]
mod tests {
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
}
