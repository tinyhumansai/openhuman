//! Platform permission detection and requests for accessibility, screen recording, input monitoring.

use super::types::{PermissionKind, PermissionState, PermissionStatus};

#[cfg(target_os = "macos")]
use std::ffi::c_void;

#[cfg(target_os = "macos")]
type CFAllocatorRef = *const c_void;
#[cfg(target_os = "macos")]
type CFDictionaryRef = *const c_void;
#[cfg(target_os = "macos")]
type CFBooleanRef = *const c_void;
#[cfg(target_os = "macos")]
type CFStringRef = *const c_void;

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFAllocatorDefault: CFAllocatorRef;
    static kCFBooleanTrue: CFBooleanRef;
    fn CFDictionaryCreate(
        allocator: CFAllocatorRef,
        keys: *const *const c_void,
        values: *const *const c_void,
        num_values: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFDictionaryRef;
    fn CFRelease(cf: *const c_void);
}

#[cfg(target_os = "macos")]
#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOHIDCheckAccess(request_type: i32) -> isize;
}

#[cfg(target_os = "macos")]
const IOHID_REQUEST_TYPE_LISTEN_EVENT: i32 = 1;
#[cfg(target_os = "macos")]
const IOHID_ACCESS_GRANTED: isize = 0;
#[cfg(target_os = "macos")]
const IOHID_ACCESS_DENIED: isize = 1;
#[cfg(target_os = "macos")]
const IOHID_ACCESS_UNKNOWN: isize = 2;

pub fn permission_to_str(permission: PermissionKind) -> &'static str {
    match permission {
        PermissionKind::ScreenRecording => "screen_recording",
        PermissionKind::Accessibility => "accessibility",
        PermissionKind::InputMonitoring => "input_monitoring",
        PermissionKind::Microphone => "microphone",
    }
}

#[cfg(target_os = "macos")]
pub fn open_macos_privacy_pane(pane: &str) {
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{pane}");
    let _ = std::process::Command::new("open").arg(url).status();
}

#[cfg(target_os = "macos")]
pub fn request_accessibility_access() {
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt as *const c_void];
        let values = [kCFBooleanTrue as *const c_void];
        let options = CFDictionaryCreate(
            kCFAllocatorDefault,
            keys.as_ptr(),
            values.as_ptr(),
            1,
            std::ptr::null(),
            std::ptr::null(),
        );
        let _ = AXIsProcessTrustedWithOptions(options);
        if !options.is_null() {
            CFRelease(options);
        }
    }
}

#[cfg(target_os = "macos")]
pub fn request_screen_recording_access() {
    unsafe {
        let _ = CGRequestScreenCaptureAccess();
    }
}

#[cfg(target_os = "macos")]
pub fn detect_accessibility_permission() -> PermissionState {
    unsafe {
        if AXIsProcessTrusted() {
            PermissionState::Granted
        } else {
            PermissionState::Denied
        }
    }
}

#[cfg(target_os = "macos")]
pub fn detect_screen_recording_permission() -> PermissionState {
    unsafe {
        if CGPreflightScreenCaptureAccess() {
            PermissionState::Granted
        } else {
            PermissionState::Denied
        }
    }
}

#[cfg(target_os = "macos")]
pub fn detect_input_monitoring_permission() -> PermissionState {
    let access = unsafe { IOHIDCheckAccess(IOHID_REQUEST_TYPE_LISTEN_EVENT) };
    match access {
        IOHID_ACCESS_GRANTED => PermissionState::Granted,
        IOHID_ACCESS_DENIED => PermissionState::Denied,
        IOHID_ACCESS_UNKNOWN => PermissionState::Unknown,
        _ => PermissionState::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Microphone permission — cross-platform
// ---------------------------------------------------------------------------

/// Detect whether the app has microphone permission.
///
/// Uses CPAL device probing as a cross-platform permission proxy:
/// - If `default_input_device()` returns a device, access is available.
/// - If it returns `None`, either permission is denied or no mic is connected.
///
/// On **macOS** under hardened runtime, CPAL will fail to enumerate input
/// devices when the `com.apple.security.device.audio-input` entitlement is
/// missing or microphone permission is denied in System Settings.
///
/// On **Windows**, `None` may indicate a privacy toggle denial or no hardware.
///
/// **Linux** standard desktops don't enforce per-app permissions; Flatpak/Snap
/// sandboxes are detected separately.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn detect_microphone_permission() -> PermissionState {
    use cpal::traits::HostTrait;
    let host = cpal::default_host();
    match host.default_input_device() {
        Some(device) => {
            let name =
                cpal::traits::DeviceTrait::name(&device).unwrap_or_else(|_| "<unknown>".into());
            log::debug!("[permissions] microphone access available — device: {name}");
            PermissionState::Granted
        }
        None => {
            log::debug!(
                "[permissions] no default input device — possible permission denial or no mic connected"
            );
            PermissionState::Unknown
        }
    }
}

#[cfg(target_os = "linux")]
pub fn detect_microphone_permission() -> PermissionState {
    // Standard Linux desktops (PulseAudio/PipeWire) don't enforce app-level mic permissions.
    // Detect Flatpak sandbox — if sandboxed, probe CPAL as a permission proxy.
    if std::env::var("FLATPAK_ID").is_ok() || std::path::Path::new("/run/flatpak").exists() {
        use cpal::traits::HostTrait;
        let host = cpal::default_host();
        match host.default_input_device() {
            Some(_) => PermissionState::Granted,
            None => {
                log::debug!(
                    "[permissions] Linux (Flatpak): no default input device — possible sandbox restriction"
                );
                PermissionState::Denied
            }
        }
    } else {
        PermissionState::Granted
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn detect_microphone_permission() -> PermissionState {
    PermissionState::Unsupported
}

/// Request microphone access from the operating system.
///
/// - **macOS**: Triggers the system permission prompt if status is `NotDetermined`.
///   Note: `AVCaptureDevice.requestAccess(for:)` is async in ObjC but we call the
///   synchronous authorization check — the system prompt is triggered by the check itself
///   when entitlements + usage description are present. Alternatively, opening the
///   Privacy pane guides the user.
/// - **Windows**: Opens the Privacy > Microphone settings page.
/// - **Linux**: No-op for standard installs; guidance for Flatpak in error messages.
#[cfg(target_os = "macos")]
pub fn request_microphone_access() {
    log::debug!("[permissions] requesting macOS microphone access via Privacy pane");
    open_macos_privacy_pane("Privacy_Microphone");
}

#[cfg(target_os = "windows")]
pub fn request_microphone_access() {
    log::debug!("[permissions] opening Windows Privacy > Microphone settings");
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "ms-settings:privacy-microphone"])
        .status();
}

#[cfg(target_os = "linux")]
pub fn request_microphone_access() {
    log::debug!("[permissions] Linux: no programmatic mic permission request available");
    // No-op — standard Linux desktops don't have an app-level permission gate.
    // For Flatpak, the XDG Portal API (ashpd crate) could be used in the future.
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn request_microphone_access() {
    // Unsupported platform — no-op.
}

/// Returns a platform-specific user-facing message when microphone permission is denied.
pub fn microphone_denied_message() -> String {
    #[cfg(target_os = "macos")]
    {
        "Microphone permission denied. Grant access in System Settings > Privacy & Security > Microphone, then restart the app.".to_string()
    }
    #[cfg(target_os = "windows")]
    {
        "Microphone access unavailable. Check Settings > Privacy & Security > Microphone and ensure the app is allowed. If no microphone is connected, plug one in.".to_string()
    }
    #[cfg(target_os = "linux")]
    {
        "No microphone device available. Check your audio settings and ensure a microphone is connected. If running in a Flatpak sandbox, grant microphone access via Flatseal or system settings.".to_string()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "Microphone access is not supported on this platform.".to_string()
    }
}

#[cfg(target_os = "macos")]
pub fn detect_permissions() -> PermissionStatus {
    PermissionStatus {
        screen_recording: detect_screen_recording_permission(),
        accessibility: detect_accessibility_permission(),
        input_monitoring: detect_input_monitoring_permission(),
        microphone: detect_microphone_permission(),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn detect_permissions() -> PermissionStatus {
    PermissionStatus {
        screen_recording: PermissionState::Unsupported,
        accessibility: PermissionState::Unsupported,
        input_monitoring: PermissionState::Unsupported,
        microphone: detect_microphone_permission(),
    }
}
