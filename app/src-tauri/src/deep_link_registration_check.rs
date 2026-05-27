//! Read-back verification for the `openhuman://` URL-scheme registration.
//!
//! `tauri-plugin-deep-link::register_all` writes
//! `HKCU\Software\Classes\openhuman\shell\open\command` on Windows so the
//! browser can hand `openhuman://auth?...` OAuth callbacks back to the running
//! desktop instance. When that write silently fails — or when the value
//! becomes stale because the install was moved out from under itself — the
//! Tauri plugin only surfaces a `warn` and the user is left with an OAuth
//! flow that never returns to the app (issue #2699).
//!
//! This module verifies the registration after `register_all` returns so the
//! actual state is logged loudly enough to be picked up by Sentry and end-user
//! support logs. We do **not** auto-repair the registry — writing the wrong
//! exe path can brick a working install — but the diagnostic surface is now
//! sufficient to point users at the documented manual repair in
//! `gitbooks/overview/troubleshooting-sign-in.md`.
//!
//! The string-parsing helpers are cross-platform so the developer host (macOS
//! / Linux) can run their unit tests; the actual registry read sits behind
//! `#[cfg(target_os = "windows")]`. The whole module is dead code on
//! non-Windows targets outside of tests, so the dead-code lint is suppressed
//! there only.

#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use std::path::Path;

/// Subkey under `HKEY_CURRENT_USER` that holds the `openhuman://` URL-scheme
/// handler command. Matches what `tauri-plugin-deep-link::register_all`
/// writes on Windows (HKCU, not HKLM, so no UAC elevation is involved).
pub(crate) const HKCU_OPEN_COMMAND_SUBKEY: &str = r"Software\Classes\openhuman\shell\open\command";

/// Outcome of inspecting the `openhuman://` protocol handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RegistrationStatus {
    /// HKCU key exists and references the running executable.
    Valid { command: String },
    /// HKCU key exists but its command points at a different exe than the one
    /// that's running. Typical after the install was moved/copied to a new
    /// path without re-running the installer.
    Stale {
        registered_command: String,
        expected_exe: String,
    },
    /// HKCU command subkey exists with no `(Default)` value or an empty one.
    MissingCommand,
    /// `HKCU\Software\Classes\openhuman` doesn't exist — the scheme has never
    /// been registered for this user.
    NotRegistered,
    /// Couldn't read the registry at all (permissions, ACL on a locked-down
    /// Windows image, transient failure).
    ReadError(String),
}

impl RegistrationStatus {
    pub(crate) fn is_healthy(&self) -> bool {
        matches!(self, Self::Valid { .. })
    }
}

/// Pull the first whitespace-delimited token out of a Windows-style command
/// string, honouring double-quoted paths. The registry stores values like
/// `"C:\Program Files\OpenHuman\OpenHuman.exe" "%1"` — we want the exe path.
pub(crate) fn extract_first_token(command: &str) -> &str {
    let trimmed = command.trim_start();
    if let Some(rest) = trimmed.strip_prefix('"') {
        match rest.find('"') {
            Some(end) => &rest[..end],
            None => rest,
        }
    } else {
        match trimmed.find(char::is_whitespace) {
            Some(end) => &trimmed[..end],
            None => trimmed,
        }
    }
}

/// Compare two Windows path strings case-insensitively after normalizing
/// directory separators. Windows treats `/` and `\` interchangeably and is
/// case-insensitive on path comparisons, so this matches OS behavior closely
/// enough to detect "registry points at the right exe."
pub(crate) fn paths_equal_loose(a: &str, b: &str) -> bool {
    fn norm(s: &str) -> String {
        s.replace('/', "\\").to_lowercase()
    }
    norm(a) == norm(b)
}

/// True iff `command` (as found in the registry) references `exe`.
pub(crate) fn command_references_exe(command: &str, exe: &Path) -> bool {
    let token = extract_first_token(command);
    paths_equal_loose(token, &exe.to_string_lossy())
}

/// Read `HKCU\Software\Classes\openhuman\shell\open\command\(Default)` and
/// classify the registration. Windows-only — all other targets get a stub
/// that returns [`RegistrationStatus::NotRegistered`] (the verification is a
/// no-op outside Windows since macOS / Linux use different mechanisms).
#[cfg(target_os = "windows")]
pub(crate) fn verify_protocol_registration() -> RegistrationStatus {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ,
    };

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(err) => {
            return RegistrationStatus::ReadError(format!("current_exe failed: {err}"));
        }
    };

    let subkey_wide: Vec<u16> = HKCU_OPEN_COMMAND_SUBKEY
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut hkey: HKEY = std::ptr::null_mut();
    // SAFETY: subkey_wide is NUL-terminated UTF-16; hkey is written iff result == 0.
    let open_result = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey_wide.as_ptr(),
            0,
            KEY_READ,
            &mut hkey,
        )
    };

    if open_result as u32 == ERROR_FILE_NOT_FOUND {
        return RegistrationStatus::NotRegistered;
    } else if open_result as u32 != ERROR_SUCCESS {
        return RegistrationStatus::ReadError(format!(
            "RegOpenKeyExW failed: win32 error {open_result}"
        ));
    }

    // Probe size first. value_name = NUL pointer would select the (Default) value,
    // but RegQueryValueExW also accepts an empty NUL-terminated string for the same.
    let value_name: [u16; 1] = [0];
    let mut value_type: u32 = 0;
    let mut needed: u32 = 0;
    // SAFETY: hkey is a valid open HKEY; we pass null for data to get the size only.
    let size_probe = unsafe {
        RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut value_type,
            std::ptr::null_mut(),
            &mut needed,
        )
    };

    if size_probe as u32 != ERROR_SUCCESS {
        // SAFETY: hkey is a valid open HKEY from the matching RegOpenKeyExW call.
        unsafe {
            RegCloseKey(hkey);
        }
        if size_probe as u32 == ERROR_FILE_NOT_FOUND {
            return RegistrationStatus::MissingCommand;
        }
        return RegistrationStatus::ReadError(format!(
            "RegQueryValueExW (size probe) failed: win32 error {size_probe}"
        ));
    }

    if value_type != REG_SZ || needed == 0 {
        // SAFETY: hkey is a valid open HKEY from the matching RegOpenKeyExW call.
        unsafe {
            RegCloseKey(hkey);
        }
        return RegistrationStatus::MissingCommand;
    }

    // `needed` is in bytes; REG_SZ uses UTF-16 so each code unit is 2 bytes. Round
    // up to accommodate values that aren't NUL-terminated on disk.
    let units = needed.div_ceil(2) as usize;
    let mut buf: Vec<u16> = vec![0u16; units];
    let mut buf_bytes: u32 = needed;
    // SAFETY: buf is sized for `needed` bytes; buf_bytes is updated by the call.
    let query_result = unsafe {
        RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut value_type,
            buf.as_mut_ptr().cast::<u8>(),
            &mut buf_bytes,
        )
    };
    // SAFETY: hkey is a valid open HKEY from the matching RegOpenKeyExW call.
    unsafe {
        RegCloseKey(hkey);
    }

    if query_result as u32 != ERROR_SUCCESS {
        return RegistrationStatus::ReadError(format!(
            "RegQueryValueExW failed: win32 error {query_result}"
        ));
    }

    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let command = String::from_utf16_lossy(&buf[..end]);
    let command_trimmed = command.trim();

    if command_trimmed.is_empty() {
        return RegistrationStatus::MissingCommand;
    }

    if command_references_exe(command_trimmed, &exe) {
        RegistrationStatus::Valid {
            command: command_trimmed.to_string(),
        }
    } else {
        RegistrationStatus::Stale {
            registered_command: command_trimmed.to_string(),
            expected_exe: exe.to_string_lossy().to_string(),
        }
    }
}

/// Non-Windows stub so the setup-time wiring can call this unconditionally
/// behind `cfg(windows)` without polluting the call site with cfg gates.
#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
pub(crate) fn verify_protocol_registration() -> RegistrationStatus {
    RegistrationStatus::NotRegistered
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn extract_first_token_quoted_exe_with_args() {
        assert_eq!(
            extract_first_token("\"C:\\Program Files\\OpenHuman\\OpenHuman.exe\" \"%1\""),
            "C:\\Program Files\\OpenHuman\\OpenHuman.exe"
        );
    }

    #[test]
    fn extract_first_token_unquoted_exe_with_args() {
        assert_eq!(
            extract_first_token("C:\\OpenHuman\\OpenHuman.exe %1"),
            "C:\\OpenHuman\\OpenHuman.exe"
        );
    }

    #[test]
    fn extract_first_token_handles_leading_whitespace() {
        assert_eq!(
            extract_first_token("   C:\\OpenHuman\\OpenHuman.exe %1"),
            "C:\\OpenHuman\\OpenHuman.exe"
        );
    }

    #[test]
    fn extract_first_token_single_value_no_args() {
        assert_eq!(extract_first_token("OpenHuman.exe"), "OpenHuman.exe");
    }

    #[test]
    fn extract_first_token_unterminated_quote_falls_through() {
        // Defensive: malformed REG_SZ should not panic. We return the rest of
        // the string instead of slicing past a missing terminator.
        assert_eq!(
            extract_first_token("\"C:\\OpenHuman\\OpenHuman.exe %1"),
            "C:\\OpenHuman\\OpenHuman.exe %1"
        );
    }

    #[test]
    fn paths_equal_loose_is_case_insensitive_and_slash_agnostic() {
        assert!(paths_equal_loose(
            "C:\\Program Files\\OpenHuman\\OpenHuman.exe",
            "c:/program files/openhuman/openhuman.exe"
        ));
    }

    #[test]
    fn paths_equal_loose_distinguishes_different_paths() {
        assert!(!paths_equal_loose(
            "C:\\OldPath\\OpenHuman.exe",
            "C:\\NewPath\\OpenHuman.exe"
        ));
    }

    #[test]
    fn command_references_exe_matches_quoted_command_with_percent_one() {
        let exe = PathBuf::from("C:\\Program Files\\OpenHuman\\OpenHuman.exe");
        assert!(command_references_exe(
            "\"C:\\Program Files\\OpenHuman\\OpenHuman.exe\" \"%1\"",
            &exe
        ));
    }

    #[test]
    fn command_references_exe_detects_stale_install_path() {
        // Repro of the "user moved the install" failure mode: registry still
        // points at the old location.
        let exe = PathBuf::from("C:\\NewLocation\\OpenHuman.exe");
        assert!(!command_references_exe(
            "\"C:\\OldLocation\\OpenHuman.exe\" \"%1\"",
            &exe
        ));
    }

    #[test]
    fn is_healthy_only_for_valid_variant() {
        assert!(RegistrationStatus::Valid {
            command: "x".into()
        }
        .is_healthy());
        assert!(!RegistrationStatus::MissingCommand.is_healthy());
        assert!(!RegistrationStatus::NotRegistered.is_healthy());
        assert!(!RegistrationStatus::Stale {
            registered_command: "x".into(),
            expected_exe: "y".into()
        }
        .is_healthy());
        assert!(!RegistrationStatus::ReadError("foo".into()).is_healthy());
    }
}
