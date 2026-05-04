//! Native keyboard control tool using enigo.
//!
//! Provides text typing, individual key presses, and hotkey combinations
//! via platform-native APIs (Core Graphics on macOS, SendInput on Windows,
//! X11/libxdo on Linux).

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
            return Ok(ToolResult::error("Action blocked: autonomy is read-only"));
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
                    .ok_or_else(|| anyhow::anyhow!("Missing 'text' for type action"))?
                    .to_string();

                if text.is_empty() {
                    return Ok(ToolResult::error("'text' cannot be empty"));
                }
                if text.len() > MAX_TYPE_LENGTH {
                    return Ok(ToolResult::error(format!(
                        "Text too long ({} chars). Maximum is {MAX_TYPE_LENGTH}.",
                        text.len()
                    )));
                }

                let len = text.len();
                tokio::task::spawn_blocking(move || {
                    let mut enigo = Enigo::new(&Settings::default())
                        .map_err(|e| anyhow::anyhow!("Failed to create enigo instance: {e}"))?;
                    enigo
                        .text(&text)
                        .map_err(|e| anyhow::anyhow!("text typing failed: {e}"))?;
                    info!(
                        tool = "keyboard",
                        action = "type",
                        chars = len,
                        "[computer] typed text"
                    );
                    Ok(ToolResult::success(format!("Typed {len} characters")))
                })
                .await?
            }

            "press" => {
                let key_name = args
                    .get("key")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("Missing 'key' for press action"))?
                    .to_string();

                let key = parse_key(&key_name).ok_or_else(|| {
                    anyhow::anyhow!("Unknown key '{key_name}'. Use names like Enter, Tab, Escape, F1-F12, a-z, 0-9, Space, etc.")
                })?;

                tokio::task::spawn_blocking(move || {
                    let mut enigo = Enigo::new(&Settings::default())
                        .map_err(|e| anyhow::anyhow!("Failed to create enigo instance: {e}"))?;
                    enigo
                        .key(key, Direction::Click)
                        .map_err(|e| anyhow::anyhow!("key press failed: {e}"))?;
                    info!(
                        tool = "keyboard",
                        action = "press",
                        key = key_name.as_str(),
                        "[computer] pressed key"
                    );
                    Ok(ToolResult::success(format!("Pressed key '{key_name}'")))
                })
                .await?
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

                if key_names.is_empty() {
                    return Ok(ToolResult::error("'keys' array cannot be empty"));
                }
                if key_names.len() > 6 {
                    return Ok(ToolResult::error(
                        "Too many keys in hotkey combination (max 6)",
                    ));
                }
                if key_names.len() < 2 {
                    return Ok(ToolResult::error(
                        "Hotkey requires at least one modifier and one final key (e.g. ['Ctrl', 'C'])",
                    ));
                }

                // Parse all key names into Key values.
                let mut keys: Vec<Key> = Vec::with_capacity(key_names.len());
                for name in &key_names {
                    let key = parse_key(name).ok_or_else(|| {
                        anyhow::anyhow!("Unknown key '{name}' in hotkey combination")
                    })?;
                    keys.push(key);
                }

                // Validate modifier-first pattern: all keys except the last
                // must be modifiers, and the last must be a non-modifier.
                let (modifiers, final_key) = keys.split_at(keys.len() - 1);
                for (i, key) in modifiers.iter().enumerate() {
                    if !is_modifier(key) {
                        return Ok(ToolResult::error(format!(
                            "Key '{}' at position {i} must be a modifier (Ctrl/Shift/Alt/Cmd). Non-modifier keys must be last.",
                            key_names[i]
                        )));
                    }
                }
                if is_modifier(&final_key[0]) {
                    return Ok(ToolResult::error(format!(
                        "Last key '{}' cannot be a modifier. Hotkey must end with a non-modifier key (e.g. 'C', 'Enter').",
                        key_names.last().unwrap()
                    )));
                }

                let combo_desc = key_names.join("+");
                tokio::task::spawn_blocking(move || {
                    let mut enigo = Enigo::new(&Settings::default())
                        .map_err(|e| anyhow::anyhow!("Failed to create enigo instance: {e}"))?;

                    // Press keys in order, tracking which were successfully
                    // pressed so we can release them on error.
                    let mut pressed_keys: Vec<Key> = Vec::with_capacity(keys.len());
                    let press_result: Result<(), anyhow::Error> = (|| {
                        for key in &keys {
                            enigo.key(*key, Direction::Press).map_err(|e| {
                                anyhow::anyhow!("key press failed for {key:?}: {e}")
                            })?;
                            pressed_keys.push(*key);
                            std::thread::sleep(HOTKEY_INTER_KEY_DELAY);
                        }
                        Ok(())
                    })();

                    // Always release all successfully pressed keys in reverse
                    // order, even if a press failed partway through.
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

                    // Now propagate any press error.
                    press_result?;

                    info!(
                        tool = "keyboard",
                        action = "hotkey",
                        combo = combo_desc.as_str(),
                        "[computer] hotkey executed"
                    );
                    Ok(ToolResult::success(format!(
                        "Executed hotkey: {combo_desc}"
                    )))
                })
                .await?
            }

            other => Ok(ToolResult::error(format!(
                "Unknown keyboard action '{other}'. Use: type, press, hotkey"
            ))),
        }
    }
}

#[cfg(test)]
#[path = "keyboard_tests.rs"]
mod tests;
