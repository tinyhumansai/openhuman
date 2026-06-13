//! Native keyboard control tool using enigo.
//!
//! Provides text typing, individual key presses, and hotkey combinations
//! via platform-native APIs (Core Graphics on macOS, SendInput on Windows,
//! X11/libxdo on Linux).

use super::main_thread::run_input_on_main;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info};

/// Small delay between key events in a hotkey sequence so the OS
/// registers each modifier correctly.
const HOTKEY_INTER_KEY_DELAY: Duration = Duration::from_millis(20);

/// Maximum text length for the `type` action to prevent accidental floods.
const MAX_TYPE_LENGTH: usize = 10_000;

pub struct KeyboardTool {
    security: Arc<SecurityPolicy>,
}

impl KeyboardTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

/// Parse a human-readable key name into an enigo `Key`.
///
/// Accepts common names (case-insensitive) plus single characters.
fn parse_key(name: &str) -> Option<Key> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        // Modifiers
        "ctrl" | "control" => Some(Key::Control),
        "shift" => Some(Key::Shift),
        "alt" | "option" => Some(Key::Alt),
        "cmd" | "command" | "meta" | "super" | "win" | "windows" => Some(Key::Meta),

        // Navigation
        "enter" | "return" => Some(Key::Return),
        "tab" => Some(Key::Tab),
        "escape" | "esc" => Some(Key::Escape),
        "backspace" => Some(Key::Backspace),
        "delete" | "del" => Some(Key::Delete),
        "space" => Some(Key::Space),

        // Arrow keys
        "up" | "arrowup" => Some(Key::UpArrow),
        "down" | "arrowdown" => Some(Key::DownArrow),
        "left" | "arrowleft" => Some(Key::LeftArrow),
        "right" | "arrowright" => Some(Key::RightArrow),

        // Home / End / Page
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "pageup" | "page_up" => Some(Key::PageUp),
        "pagedown" | "page_down" => Some(Key::PageDown),

        // Function keys
        "f1" => Some(Key::F1),
        "f2" => Some(Key::F2),
        "f3" => Some(Key::F3),
        "f4" => Some(Key::F4),
        "f5" => Some(Key::F5),
        "f6" => Some(Key::F6),
        "f7" => Some(Key::F7),
        "f8" => Some(Key::F8),
        "f9" => Some(Key::F9),
        "f10" => Some(Key::F10),
        "f11" => Some(Key::F11),
        "f12" => Some(Key::F12),

        // Caps Lock
        "capslock" | "caps_lock" => Some(Key::CapsLock),

        // Single character — letters, digits, punctuation
        _ => {
            let chars: Vec<char> = name.chars().collect();
            if chars.len() == 1 {
                Some(Key::Unicode(chars[0]))
            } else {
                None
            }
        }
    }
}

/// Returns true if the key is a modifier (Ctrl, Shift, Alt, Meta).
fn is_modifier(key: &Key) -> bool {
    matches!(key, Key::Control | Key::Shift | Key::Alt | Key::Meta)
}

// ── Shared execution helpers ────────────────────────────────────────────────
// These hold the validation + main-thread enigo dispatch once, so both the
// `keyboard` tool and the `automate` backend (`RealBackend::{key,type_text}`)
// drive synthetic input through a single code path. They return the raw
// `Result<String, String>`; the tool wraps it with `into_result`.

/// Type a literal string via the OS. Runs on the app main thread (macOS TSM
/// requirement, Change 1.15). Validates length like the `type` tool action.
pub(crate) async fn run_type_text(text: &str) -> Result<String, String> {
    if text.is_empty() {
        return Err("'text' cannot be empty".to_string());
    }
    if text.len() > MAX_TYPE_LENGTH {
        return Err(format!(
            "Text too long ({} chars). Maximum is {MAX_TYPE_LENGTH}.",
            text.len()
        ));
    }
    let text = text.to_string();
    let len = text.len();
    run_input_on_main(move || {
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Failed to create enigo instance: {e}"))?;
        enigo
            .text(&text)
            .map_err(|e| format!("text typing failed: {e}"))?;
        Ok(format!("Typed {len} characters"))
    })
    .await
}

/// Tap a single key by name (e.g. "Enter", "/", "k"). Used for app shortcuts
/// like YouTube's `/` (focus search) or `k` (play/pause).
pub(crate) async fn run_key(key_name: &str) -> Result<String, String> {
    let key = parse_key(key_name).ok_or_else(|| {
        format!(
            "Unknown key '{key_name}'. Use names like Enter, Tab, Escape, F1-F12, a-z, 0-9, Space, etc."
        )
    })?;
    let key_name = key_name.to_string();
    run_input_on_main(move || {
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Failed to create enigo instance: {e}"))?;
        enigo
            .key(key, Direction::Click)
            .map_err(|e| format!("key press failed: {e}"))?;
        Ok(format!("Pressed key '{key_name}'"))
    })
    .await
}

/// Execute a hotkey chord — modifiers first, then exactly one non-modifier
/// final key (e.g. `["Cmd","L"]`). Validates the modifier-first shape, then
/// presses in order and releases in reverse (even on error).
pub(crate) async fn run_hotkey(key_names: &[String]) -> Result<String, String> {
    if key_names.is_empty() {
        return Err("'keys' array cannot be empty".to_string());
    }
    if key_names.len() > 6 {
        return Err("Too many keys in hotkey combination (max 6)".to_string());
    }
    if key_names.len() < 2 {
        return Err(
            "Hotkey requires at least one modifier and one final key (e.g. ['Ctrl', 'C'])"
                .to_string(),
        );
    }

    let mut keys: Vec<Key> = Vec::with_capacity(key_names.len());
    for name in key_names {
        let key =
            parse_key(name).ok_or_else(|| format!("Unknown key '{name}' in hotkey combination"))?;
        keys.push(key);
    }

    // Validate modifier-first: all but the last must be modifiers; the last
    // must be a non-modifier.
    let (modifiers, final_key) = keys.split_at(keys.len() - 1);
    for (i, key) in modifiers.iter().enumerate() {
        if !is_modifier(key) {
            return Err(format!(
                "Key '{}' at position {i} must be a modifier (Ctrl/Shift/Alt/Cmd). Non-modifier keys must be last.",
                key_names[i]
            ));
        }
    }
    if is_modifier(&final_key[0]) {
        return Err(format!(
            "Last key '{}' cannot be a modifier. Hotkey must end with a non-modifier key (e.g. 'C', 'Enter').",
            key_names.last().unwrap()
        ));
    }

    let combo_desc = key_names.join("+");
    run_input_on_main(move || {
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Failed to create enigo instance: {e}"))?;

        // Press keys in order, tracking which were pressed so we can release
        // them on error.
        let mut pressed_keys: Vec<Key> = Vec::with_capacity(keys.len());
        let press_result: Result<(), String> = (|| {
            for key in &keys {
                enigo
                    .key(*key, Direction::Press)
                    .map_err(|e| format!("key press failed for {key:?}: {e}"))?;
                pressed_keys.push(*key);
                std::thread::sleep(HOTKEY_INTER_KEY_DELAY);
            }
            Ok(())
        })();

        // Always release pressed keys in reverse, even on error.
        for key in pressed_keys.iter().rev() {
            if let Err(e) = enigo.key(*key, Direction::Release) {
                tracing::warn!(
                    tool = "keyboard",
                    key = ?key,
                    error = %e,
                    "[computer] best-effort key release failed during cleanup"
                );
            }
        }
        press_result?;
        Ok(format!("Executed hotkey: {combo_desc}"))
    })
    .await
}

#[async_trait]
impl Tool for KeyboardTool {
    fn name(&self) -> &str {
        "keyboard"
    }

    fn description(&self) -> &str {
        concat!(
            "Simulate keyboard input natively. Actions: type (enter a text string), ",
            "press (tap a single key like Enter or Tab), hotkey (key combination like ",
            "Ctrl+C or Cmd+Shift+S). Key names are case-insensitive."
        )
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    /// Route every call through the ApprovalGate. Arbitrary keystrokes can land
    /// in a focused sudo/password field or Terminal, and there's no sensitive-app
    /// denylist for raw input, so the gate is the only boundary — and it fires on
    /// `external_effect_with_args`, NOT on `PermissionLevel::Dangerous`. Without
    /// this, keystrokes could run unattended on an auto-approved turn once
    /// `computer_control.enabled`.
    fn external_effect(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["type", "press", "hotkey"],
                    "description": "Keyboard action to perform"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type. Required for 'type' action. Max 10,000 chars."
                },
                "key": {
                    "type": "string",
                    "description": "Key name (e.g. 'Enter', 'Tab', 'Escape', 'a', 'F5'). Required for 'press' action."
                },
                "keys": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Key combination as ordered array. Modifiers first, then the final key (e.g. ['Ctrl', 'C'] or ['Cmd', 'Shift', 'S']). Required for 'hotkey' action."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        if !self.security.can_act() {
            debug!(
                tool = "keyboard",
                "[computer] blocked: autonomy is read-only"
            );
            return Ok(ToolResult::error(
                "[policy-blocked] Action blocked: autonomy is read-only",
            ));
        }
        if !self.security.record_action() {
            debug!(tool = "keyboard", "[computer] blocked: rate limit exceeded");
            return Ok(ToolResult::error("Action blocked: rate limit exceeded"));
        }

        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        debug!(
            tool = "keyboard",
            action = action,
            "[computer] keyboard action requested"
        );

        match action {
            "type" => {
                let text = args
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("Missing 'text' for type action"))?;
                into_result("type", run_type_text(text).await)
            }

            "press" => {
                let key_name = args
                    .get("key")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("Missing 'key' for press action"))?;
                into_result("press", run_key(key_name).await)
            }

            "hotkey" => {
                let raw_keys = args
                    .get("keys")
                    .and_then(Value::as_array)
                    .ok_or_else(|| anyhow::anyhow!("Missing 'keys' array for hotkey action"))?;

                // Reject non-string entries up front.
                let mut key_names: Vec<String> = Vec::with_capacity(raw_keys.len());
                for (i, v) in raw_keys.iter().enumerate() {
                    let s = v.as_str().ok_or_else(|| {
                        anyhow::anyhow!("Element {i} in 'keys' array is not a string (got {v})")
                    })?;
                    key_names.push(s.to_string());
                }

                into_result("hotkey", run_hotkey(&key_names).await)
            }

            other => Ok(ToolResult::error(format!(
                "Unknown keyboard action '{other}'. Use: type, press, hotkey"
            ))),
        }
    }
}

/// Map a main-thread input op result to a `ToolResult`, logging the outcome.
fn into_result(action: &str, r: Result<String, String>) -> anyhow::Result<ToolResult> {
    match r {
        Ok(msg) => {
            info!(tool = "keyboard", action, "[computer] {msg}");
            Ok(ToolResult::success(msg))
        }
        Err(e) => {
            tracing::warn!(tool = "keyboard", action, "[computer] failed: {e}");
            Ok(ToolResult::error(e))
        }
    }
}

#[cfg(test)]
#[path = "keyboard_tests.rs"]
mod tests;
