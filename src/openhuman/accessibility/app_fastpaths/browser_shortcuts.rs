//! Cross-platform browser keyboard-shortcut table (Change 1.17).
//!
//! The four major desktop browsers (Chrome, Firefox, Safari, Edge) share almost
//! the same navigation/tab/find shortcuts — they differ mainly by the primary
//! modifier (⌘ on macOS, Ctrl on Windows/Linux), with a handful of real
//! per-browser exceptions (Firefox's `Alt+1‑8` tab selection on Linux, its
//! `Ctrl/⌘+Shift+P` private window, per-browser History/Downloads keys, etc.).
//!
//! [`shortcut`] resolves `(intent, browser, os)` to a key chord expressed as the
//! same key names the keyboard tool's `parse_key` understands (`"Cmd"`,
//! `"Ctrl"`, `"Shift"`, `"Alt"`, single chars, `"left"`, `"tab"`, `"f11"`, …),
//! so the result feeds straight into `AutomateBackend::key`.
//!
//! Sources: Chrome/Firefox/Edge support pages (Win/macOS/Linux columns) and the
//! Safari for Mac guide. In-page media shortcuts (YouTube `k`/`f`/`m`/`Shift+N`)
//! are browser-independent and live in `browser.rs`, not here.

/// Target operating system — selects the primary modifier and the non-Mac
/// fallbacks (Alt+arrow history, F11 fullscreen, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

impl Os {
    /// The OS this build is running on.
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// Browser family — chosen by [`Browser::from_display`]. Chromium-based browsers
/// (Brave, Arc, Chromium) share Chrome's shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    Chrome,
    Firefox,
    Safari,
    Edge,
}

impl Browser {
    /// Map a macOS display name (or alias) to a family. Unknown / generic names
    /// default to Chrome, since most other desktop browsers are Chromium-based
    /// and share its bindings.
    pub fn from_display(name: &str) -> Browser {
        let n = name.to_lowercase();
        if n.contains("firefox") {
            Browser::Firefox
        } else if n.contains("safari") {
            Browser::Safari
        } else if n.contains("edge") {
            Browser::Edge
        } else {
            // Chrome, Brave, Arc, Chromium, Vivaldi, Opera, … → Chrome bindings.
            Browser::Chrome
        }
    }
}

/// A browser action we can trigger with a keyboard shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserShortcut {
    FocusAddressBar,
    NewTab,
    CloseTab,
    ReopenTab,
    NewWindow,
    PrivateWindow,
    NextTab,
    PrevTab,
    /// Jump to tab N (1-based, 1‑8). N≥9 is treated as the last tab.
    TabN(u8),
    LastTab,
    Find,
    FindNext,
    FindPrev,
    Reload,
    HardReload,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Print,
    BookmarkPage,
    Back,
    Forward,
    History,
    Downloads,
    Fullscreen,
}

/// The primary modifier: ⌘ on macOS, Ctrl elsewhere.
fn primary(os: Os) -> &'static str {
    match os {
        Os::Mac => "Cmd",
        _ => "Ctrl",
    }
}

fn keys(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Resolve a shortcut to a key chord for the given browser + OS.
pub fn shortcut(intent: BrowserShortcut, browser: Browser, os: Os) -> Vec<String> {
    use BrowserShortcut as S;
    let p = primary(os);
    match intent {
        // ── uniform across all four browsers (only ⌘↔Ctrl differs) ──
        S::FocusAddressBar => keys(&[p, "l"]),
        S::NewTab => keys(&[p, "t"]),
        S::CloseTab => keys(&[p, "w"]),
        S::ReopenTab => keys(&[p, "shift", "t"]),
        S::NewWindow => keys(&[p, "n"]),
        // Ctrl+Tab cycles tabs on every OS/browser (not ⌘ on macOS).
        S::NextTab => keys(&["ctrl", "tab"]),
        S::PrevTab => keys(&["ctrl", "shift", "tab"]),
        S::Find => keys(&[p, "f"]),
        S::FindNext => keys(&[p, "g"]),
        S::FindPrev => keys(&[p, "shift", "g"]),
        S::Reload => keys(&[p, "r"]),
        S::HardReload => keys(&[p, "shift", "r"]),
        // Use "=" (Ctrl/⌘+=) for zoom-in: it's the unshifted key and every
        // browser accepts it as zoom-in, avoiding the Shift needed to type "+".
        S::ZoomIn => keys(&[p, "="]),
        S::ZoomOut => keys(&[p, "-"]),
        S::ZoomReset => keys(&[p, "0"]),
        S::Print => keys(&[p, "p"]),
        S::BookmarkPage => keys(&[p, "d"]),

        // ── per-browser / per-OS exceptions ──
        S::PrivateWindow => match browser {
            Browser::Firefox => keys(&[p, "shift", "p"]),
            _ => keys(&[p, "shift", "n"]),
        },
        S::TabN(n) => {
            let d = n.clamp(1, 8).to_string();
            // Firefox on Linux uses Alt+1‑8 (not Ctrl).
            if browser == Browser::Firefox && os == Os::Linux {
                keys(&["alt", &d])
            } else {
                keys(&[p, &d])
            }
        }
        S::LastTab => {
            if browser == Browser::Firefox && os == Os::Linux {
                keys(&["alt", "9"])
            } else {
                keys(&[p, "9"])
            }
        }
        S::Back => match os {
            Os::Mac => keys(&[p, "["]),
            _ => keys(&["alt", "left"]),
        },
        S::Forward => match os {
            Os::Mac => keys(&[p, "]"]),
            _ => keys(&["alt", "right"]),
        },
        S::History => match os {
            Os::Mac => match browser {
                // Chrome/Edge: ⌘Y. Firefox/Safari: ⌘⇧H.
                Browser::Chrome | Browser::Edge => keys(&[p, "y"]),
                Browser::Firefox | Browser::Safari => keys(&[p, "shift", "h"]),
            },
            _ => keys(&["ctrl", "h"]),
        },
        S::Downloads => match os {
            Os::Mac => match browser {
                Browser::Chrome => keys(&[p, "shift", "j"]),
                Browser::Firefox => keys(&[p, "j"]),
                // Edge / Safari: ⌥⌘L.
                Browser::Edge | Browser::Safari => keys(&[p, "option", "l"]),
            },
            _ => keys(&["ctrl", "j"]),
        },
        S::Fullscreen => match os {
            // Avoid the Fn key: ⌃⌘F is the standard macOS window fullscreen
            // (Chrome/Edge/Safari); Firefox uses ⌘⇧F.
            Os::Mac => match browser {
                Browser::Firefox => keys(&[p, "shift", "f"]),
                _ => keys(&["ctrl", "cmd", "f"]),
            },
            _ => keys(&["f11"]),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use BrowserShortcut as S;

    #[test]
    fn primary_modifier_per_os() {
        assert_eq!(
            shortcut(S::FocusAddressBar, Browser::Chrome, Os::Mac),
            vec!["Cmd", "l"]
        );
        assert_eq!(
            shortcut(S::FocusAddressBar, Browser::Chrome, Os::Windows),
            vec!["Ctrl", "l"]
        );
        assert_eq!(
            shortcut(S::FocusAddressBar, Browser::Safari, Os::Mac),
            vec!["Cmd", "l"]
        );
    }

    #[test]
    fn uniform_core_shortcuts() {
        assert_eq!(
            shortcut(S::NewTab, Browser::Edge, Os::Linux),
            vec!["Ctrl", "t"]
        );
        assert_eq!(
            shortcut(S::ReopenTab, Browser::Firefox, Os::Mac),
            vec!["Cmd", "shift", "t"]
        );
        assert_eq!(
            shortcut(S::HardReload, Browser::Chrome, Os::Windows),
            vec!["Ctrl", "shift", "r"]
        );
        // Next/prev tab is Ctrl-based on every OS, even macOS.
        assert_eq!(
            shortcut(S::NextTab, Browser::Chrome, Os::Mac),
            vec!["ctrl", "tab"]
        );
    }

    #[test]
    fn private_window_firefox_differs() {
        assert_eq!(
            shortcut(S::PrivateWindow, Browser::Firefox, Os::Windows),
            vec!["Ctrl", "shift", "p"]
        );
        assert_eq!(
            shortcut(S::PrivateWindow, Browser::Chrome, Os::Windows),
            vec!["Ctrl", "shift", "n"]
        );
    }

    #[test]
    fn tab_n_firefox_linux_uses_alt() {
        assert_eq!(
            shortcut(S::TabN(3), Browser::Firefox, Os::Linux),
            vec!["alt", "3"]
        );
        assert_eq!(
            shortcut(S::TabN(3), Browser::Chrome, Os::Linux),
            vec!["Ctrl", "3"]
        );
        assert_eq!(
            shortcut(S::TabN(3), Browser::Firefox, Os::Mac),
            vec!["Cmd", "3"]
        );
        // Out-of-range clamps into 1‑8.
        assert_eq!(
            shortcut(S::TabN(20), Browser::Chrome, Os::Mac),
            vec!["Cmd", "8"]
        );
        assert_eq!(
            shortcut(S::LastTab, Browser::Firefox, Os::Linux),
            vec!["alt", "9"]
        );
    }

    #[test]
    fn back_forward_os_specific() {
        assert_eq!(
            shortcut(S::Back, Browser::Chrome, Os::Mac),
            vec!["Cmd", "["]
        );
        assert_eq!(
            shortcut(S::Back, Browser::Chrome, Os::Windows),
            vec!["alt", "left"]
        );
        assert_eq!(
            shortcut(S::Forward, Browser::Safari, Os::Mac),
            vec!["Cmd", "]"]
        );
    }

    #[test]
    fn history_and_downloads_per_browser_on_mac() {
        assert_eq!(
            shortcut(S::History, Browser::Chrome, Os::Mac),
            vec!["Cmd", "y"]
        );
        assert_eq!(
            shortcut(S::History, Browser::Firefox, Os::Mac),
            vec!["Cmd", "shift", "h"]
        );
        assert_eq!(
            shortcut(S::History, Browser::Edge, Os::Windows),
            vec!["ctrl", "h"]
        );
        assert_eq!(
            shortcut(S::Downloads, Browser::Chrome, Os::Mac),
            vec!["Cmd", "shift", "j"]
        );
        assert_eq!(
            shortcut(S::Downloads, Browser::Firefox, Os::Mac),
            vec!["Cmd", "j"]
        );
        assert_eq!(
            shortcut(S::Downloads, Browser::Edge, Os::Mac),
            vec!["Cmd", "option", "l"]
        );
        assert_eq!(
            shortcut(S::Downloads, Browser::Chrome, Os::Linux),
            vec!["ctrl", "j"]
        );
    }

    #[test]
    fn fullscreen_avoids_fn_key() {
        assert_eq!(
            shortcut(S::Fullscreen, Browser::Chrome, Os::Mac),
            vec!["ctrl", "cmd", "f"]
        );
        assert_eq!(
            shortcut(S::Fullscreen, Browser::Firefox, Os::Mac),
            vec!["Cmd", "shift", "f"]
        );
        assert_eq!(
            shortcut(S::Fullscreen, Browser::Chrome, Os::Windows),
            vec!["f11"]
        );
    }

    #[test]
    fn from_display_maps_families() {
        assert_eq!(Browser::from_display("Brave Browser"), Browser::Chrome);
        assert_eq!(Browser::from_display("Arc"), Browser::Chrome);
        assert_eq!(Browser::from_display("Google Chrome"), Browser::Chrome);
        assert_eq!(Browser::from_display("Firefox"), Browser::Firefox);
        assert_eq!(Browser::from_display("Microsoft Edge"), Browser::Edge);
        assert_eq!(Browser::from_display("Safari"), Browser::Safari);
        // Unknown → Chrome default.
        assert_eq!(Browser::from_display("Some New Browser"), Browser::Chrome);
    }
}
