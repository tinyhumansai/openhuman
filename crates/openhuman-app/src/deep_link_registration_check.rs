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
//! `docs/gitbooks/en/overview/troubleshooting-sign-in.md`.
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

    /// Render the status as a single-line string with full filesystem paths
    /// reduced to just their final component. The `Stale` and `Valid`
    /// variants carry registry / current-exe paths that on Windows include
    /// `C:\Users\<username>\AppData\Local\...` for per-user installs — that
    /// username lands in Sentry / user log lines unless we strip it. We keep
    /// the basename so the diagnostic still tells the reader *what* exe is
    /// registered, just not *where*. Used in place of `Debug` at the log
    /// call site.
    pub(crate) fn redacted(&self) -> String {
        match self {
            Self::Valid { command } => {
                format!("Valid {{ exe: {} }}", basename_of_first_token(command))
            }
            Self::Stale {
                registered_command,
                expected_exe,
            } => format!(
                "Stale {{ registered_exe: {}, expected_exe: {} }}",
                basename_of_first_token(registered_command),
                basename_of_first_token(expected_exe),
            ),
            Self::MissingCommand => "MissingCommand".into(),
            Self::NotRegistered => "NotRegistered".into(),
            // `ReadError` carries a win32-error string like
            // "RegOpenKeyExW failed: win32 error 5" — no user paths.
            Self::ReadError(msg) => format!("ReadError({msg})"),
        }
    }
}

/// Take the first whitespace-delimited token out of `s` (handling quoted
/// command strings via [`extract_first_token`]) and return only its file
/// name. Drops the directory component so the log line doesn't leak the
/// running user's install path.
///
/// We scan for `\\` and `/` manually rather than using [`Path::file_name`]
/// because `std::path::Path` uses **host-OS** separator semantics, so on a
/// macOS / Linux dev host a Windows-style `"C:\\foo\\bar.exe"` would come
/// back as a single component and defeat the redaction.
fn basename_of_first_token(s: &str) -> String {
    let token = extract_first_token(s);
    token
        .rsplit(['\\', '/'])
        .next()
        .filter(|seg| !seg.is_empty())
        .unwrap_or("<redacted>")
        .to_string()
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

    // RAII wrapper for the open HKEY. Mirrors the `OwnedMutex` pattern used
    // in `lib.rs::run()` for the pre-CEF mutex handle: any early return after
    // the first successful `RegOpenKeyExW` falls through Drop instead of
    // having to remember to call `RegCloseKey` on every branch.
    struct OwnedHkey(HKEY);
    impl Drop for OwnedHkey {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: self.0 is only set via a successful `RegOpenKeyExW`
                // and is not aliased elsewhere — this Drop is the sole closer.
                unsafe { RegCloseKey(self.0) };
            }
        }
    }

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
    // Start null so Drop is a no-op if `RegOpenKeyExW` never succeeds.
    let mut hkey = OwnedHkey(std::ptr::null_mut());
    // SAFETY: subkey_wide is NUL-terminated UTF-16; hkey.0 is written iff result == 0.
    let open_result = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey_wide.as_ptr(),
            0,
            KEY_READ,
            &mut hkey.0,
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
    // SAFETY: hkey.0 is a valid open HKEY; we pass null for data to get the size only.
    let size_probe = unsafe {
        RegQueryValueExW(
            hkey.0,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut value_type,
            std::ptr::null_mut(),
            &mut needed,
        )
    };

    if size_probe as u32 != ERROR_SUCCESS {
        if size_probe as u32 == ERROR_FILE_NOT_FOUND {
            return RegistrationStatus::MissingCommand;
        }
        return RegistrationStatus::ReadError(format!(
            "RegQueryValueExW (size probe) failed: win32 error {size_probe}"
        ));
    }

    if value_type != REG_SZ || needed == 0 {
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
            hkey.0,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut value_type,
            buf.as_mut_ptr().cast::<u8>(),
            &mut buf_bytes,
        )
    };

    // From here on we no longer need the handle; Drop closes it at function exit.

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
#[path = "deep_link_registration_check_tests.rs"]
mod tests;
