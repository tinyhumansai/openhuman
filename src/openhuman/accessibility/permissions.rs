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

#[cfg(target_os = "macos")]
pub fn detect_permissions() -> PermissionStatus {
    PermissionStatus {
        screen_recording: detect_screen_recording_permission(),
        accessibility: detect_accessibility_permission(),
        input_monitoring: detect_input_monitoring_permission(),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn detect_permissions() -> PermissionStatus {
    PermissionStatus {
        screen_recording: PermissionState::Unsupported,
        accessibility: PermissionState::Unsupported,
        input_monitoring: PermissionState::Unsupported,
    }
}
