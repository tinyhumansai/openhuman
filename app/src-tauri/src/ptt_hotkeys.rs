//! Global push-to-talk hotkey state + parsing.
//!
//! See spec: `docs/superpowers/specs/2026-06-02-global-ptt-design.md`.
//!
//! `expand_ptt_shortcuts` mirrors `dictation_hotkeys::expand_dictation_shortcuts`
//! but rejects pure-modifier shortcuts (Ctrl, Cmd+Shift, etc.) because they
//! would fire constantly during normal typing.

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Mutex;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PttError {
    EmptyShortcut,
    ModifierOnlyShortcut,
    ConflictsWithDictation(String),
    UnsupportedOnWayland,
    RegistrationFailed(String),
}

impl std::fmt::Display for PttError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PttError::EmptyShortcut => write!(f, "ptt shortcut cannot be empty"),
            PttError::ModifierOnlyShortcut => write!(
                f,
                "ptt shortcut cannot be only modifier keys (Ctrl/Cmd/Shift/Alt)"
            ),
            PttError::ConflictsWithDictation(s) => {
                write!(f, "ptt shortcut '{s}' conflicts with the dictation hotkey")
            }
            PttError::UnsupportedOnWayland => write!(
                f,
                "global shortcuts are not supported in this Wayland session — switch to X11 or use in-app dictation"
            ),
            PttError::RegistrationFailed(s) => {
                write!(f, "failed to register ptt shortcut: {s}")
            }
        }
    }
}

impl std::error::Error for PttError {}

/// Process-wide PTT state. Held in the Tauri-managed `State<PttHotkeyState>`.
pub(crate) struct PttHotkeyState {
    /// Currently-registered shortcut variants (e.g. `["Cmd+F13", "Ctrl+F13"]` on macOS).
    pub(crate) shortcut: Mutex<Vec<String>>,
    /// Monotonic counter for session IDs.
    pub(crate) session_counter: AtomicU64,
    /// CAS-guarded: true iff a PTT session is currently mid-hold.
    /// Used to drop OS key-repeat Pressed events so each press/release pair
    /// produces exactly one session_id.
    pub(crate) is_held: AtomicBool,
}

impl PttHotkeyState {
    pub(crate) fn new() -> Self {
        Self {
            shortcut: Mutex::new(Vec::new()),
            session_counter: AtomicU64::new(0),
            is_held: AtomicBool::new(false),
        }
    }
}

const MODIFIER_TOKENS: &[&str] = &[
    "ctrl",
    "control",
    "cmd",
    "command",
    "meta",
    "super",
    "win",
    "windows",
    "alt",
    "option",
    "shift",
    "cmdorctrl",
];

fn is_modifier_token(token: &str) -> bool {
    let trimmed = token.trim();
    MODIFIER_TOKENS
        .iter()
        .any(|m| trimmed.eq_ignore_ascii_case(m))
}

/// Expand a user-typed shortcut into one or two OS-specific variants and
/// validate it isn't empty / modifier-only.
pub(crate) fn expand_ptt_shortcuts(shortcut: &str) -> Result<Vec<String>, PttError> {
    let trimmed = shortcut.trim();
    if trimmed.is_empty() {
        return Err(PttError::EmptyShortcut);
    }

    let parts: Vec<&str> = trimmed.split('+').map(str::trim).collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(PttError::EmptyShortcut);
    }
    if parts.iter().all(|p| is_modifier_token(p)) {
        return Err(PttError::ModifierOnlyShortcut);
    }

    #[cfg(target_os = "macos")]
    {
        if trimmed.contains("CmdOrCtrl") {
            let cmd_variant = trimmed.replace("CmdOrCtrl", "Cmd");
            let ctrl_variant = trimmed.replace("CmdOrCtrl", "Ctrl");
            if cmd_variant == ctrl_variant {
                return Ok(vec![cmd_variant]);
            }
            return Ok(vec![cmd_variant, ctrl_variant]);
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        if trimmed.contains("CmdOrCtrl") {
            return Ok(vec![trimmed.replace("CmdOrCtrl", "Ctrl")]);
        }
    }

    Ok(vec![trimmed.to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_shortcut_is_rejected() {
        assert_eq!(expand_ptt_shortcuts(""), Err(PttError::EmptyShortcut));
        assert_eq!(expand_ptt_shortcuts("   "), Err(PttError::EmptyShortcut));
    }

    #[test]
    fn modifier_only_shortcut_is_rejected() {
        assert_eq!(
            expand_ptt_shortcuts("Ctrl"),
            Err(PttError::ModifierOnlyShortcut)
        );
        assert_eq!(
            expand_ptt_shortcuts("Cmd+Shift"),
            Err(PttError::ModifierOnlyShortcut)
        );
        assert_eq!(
            expand_ptt_shortcuts("Alt+Shift+Ctrl"),
            Err(PttError::ModifierOnlyShortcut)
        );
        assert_eq!(
            expand_ptt_shortcuts("CmdOrCtrl+Shift"),
            Err(PttError::ModifierOnlyShortcut)
        );
    }

    #[test]
    fn plain_function_key_is_accepted() {
        assert_eq!(expand_ptt_shortcuts("F13"), Ok(vec!["F13".to_string()]));
    }

    #[test]
    fn modifier_plus_letter_is_accepted() {
        assert_eq!(
            expand_ptt_shortcuts("Ctrl+Alt+T"),
            Ok(vec!["Ctrl+Alt+T".to_string()])
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn cmd_or_ctrl_expands_to_both_on_macos() {
        let result = expand_ptt_shortcuts("CmdOrCtrl+Shift+P").unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"Cmd+Shift+P".to_string()));
        assert!(result.contains(&"Ctrl+Shift+P".to_string()));
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn cmd_or_ctrl_expands_to_ctrl_off_macos() {
        let result = expand_ptt_shortcuts("CmdOrCtrl+Shift+P").unwrap();
        assert_eq!(result, vec!["Ctrl+Shift+P".to_string()]);
    }

    #[test]
    fn malformed_shortcut_with_empty_tokens_is_rejected() {
        assert_eq!(expand_ptt_shortcuts("+F13"), Err(PttError::EmptyShortcut));
        assert_eq!(expand_ptt_shortcuts("F13+"), Err(PttError::EmptyShortcut));
        assert_eq!(
            expand_ptt_shortcuts("Ctrl++T"),
            Err(PttError::EmptyShortcut)
        );
    }
}

/// Returns `Some(conflicting_variant)` if any expanded PTT variant overlaps
/// any expanded dictation variant. Comparison is case-insensitive.
pub(crate) fn first_conflict_with(ptt: &[String], dictation: &[String]) -> Option<String> {
    for p in ptt {
        let p_lc = p.to_ascii_lowercase();
        for d in dictation {
            if d.to_ascii_lowercase() == p_lc {
                return Some(p.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod conflict_tests {
    use super::*;

    #[test]
    fn no_conflict_returns_none() {
        let ptt = vec!["F13".into()];
        let dict = vec!["F14".into()];
        assert_eq!(first_conflict_with(&ptt, &dict), None);
    }

    #[test]
    fn case_insensitive_conflict_detected() {
        let ptt = vec!["ctrl+space".into()];
        let dict = vec!["Ctrl+Space".into()];
        assert_eq!(
            first_conflict_with(&ptt, &dict),
            Some("ctrl+space".to_string())
        );
    }

    #[test]
    fn only_one_variant_overlaps_returns_first() {
        let ptt = vec!["Cmd+P".into(), "Ctrl+P".into()];
        let dict = vec!["Ctrl+P".into()];
        assert_eq!(first_conflict_with(&ptt, &dict), Some("Ctrl+P".to_string()));
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn new_state_is_not_held_and_counter_is_zero() {
        let s = PttHotkeyState::new();
        assert!(!s.is_held.load(Ordering::Relaxed));
        assert_eq!(s.session_counter.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn cas_false_to_true_succeeds_then_repeat_fails() {
        let s = PttHotkeyState::new();
        // First press: false → true succeeds.
        assert!(
            s.is_held
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "first press CAS should succeed"
        );
        // Repeat press: false → true fails because we're already true.
        assert!(
            s.is_held
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err(),
            "repeat press CAS should fail (already held)"
        );
        // Release: swap true → false returns the old true.
        assert!(
            s.is_held.swap(false, Ordering::AcqRel),
            "swap should return prior true"
        );
        // Subsequent stale release: swap returns the current false.
        assert!(
            !s.is_held.swap(false, Ordering::AcqRel),
            "stale swap should return false"
        );
    }
}
