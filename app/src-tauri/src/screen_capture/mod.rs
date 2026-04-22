//! Screen-capture source enumeration + picker orchestration for #713.
//!
//! Background (see issue #713 plan): embedded webviews (Meet, Discord, Zoom)
//! run under the CEF Alloy runtime, which does not link Chromium's built-in
//! `DesktopMediaPicker`. When the page calls `navigator.mediaDevices
//! .getDisplayMedia`, Chromium falls back to auto-selecting the primary
//! display — the user never sees a picker and their whole screen streams.
//!
//! Our `OnRequestMediaAccessPermission` callback in tauri-cef grants the
//! `DESKTOP_VIDEO_CAPTURE` bit unconditionally. Stage 0 PoC proved that when
//! the page calls `getUserMedia` with a hand-crafted
//! `{ mandatory: { chromeMediaSource: 'desktop', chromeMediaSourceId: '<id>' } }`
//! constraint, Chromium honours the ID and opens a real capture device —
//! even though this constraint shape is normally extension-only.
//!
//! This module is the host-side half of that flow:
//!   * `screen_share_list_sources` — enumerate real screens and windows,
//!     tag each with a Chromium-compatible `DesktopMediaID` string
//!     (`screen:<CGDirectDisplayID>:0` / `window:<CGWindowID>:0`).
//!   * `screen_share_thumbnail` — capture a single source's live thumbnail
//!     as a base64 PNG. Called lazily per-source from the picker shim so
//!     the picker UI opens immediately and thumbnails fade in as they
//!     arrive, rather than blocking enumeration for ~1-2s on a many-
//!     window desktop.
//!
//! The picker UI itself is injected directly into each child webview's
//! DOM by `webview_accounts/runtime.js` (see the `showInPagePicker` flow
//! there), which is why we only need IPCs for enumeration + thumbnail
//! capture and no picker-modal orchestration RPCs on the host side.
//!
//! macOS-first: other platforms stub out until the flow is proven end-
//! to-end.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenSource {
    /// `screen:<CGDirectDisplayID>:0` or `window:<CGWindowID>:0`. Chromium's
    /// `DesktopMediaID::Parse` reads these directly; we rely on its existing
    /// parser rather than round-tripping through the extension API.
    pub id: String,
    /// `"screen"` or `"window"`.
    pub kind: String,
    /// Human label shown in the picker (app name + window title, or display
    /// name).
    pub name: String,
    /// Optional application name (windows only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    /// PNG thumbnail base64-encoded. Empty when enumeration cheap-path is
    /// used — UI renders a placeholder.
    #[serde(default)]
    pub thumbnail_png_base64: String,
}

// ---------------------------------------------------------------------------
// Enumeration
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn screen_share_list_sources() -> Result<Vec<ScreenSource>, String> {
    #[cfg(target_os = "macos")]
    {
        macos::enumerate().map_err(|e| format!("enumerate failed: {e}"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("screen-share picker not implemented for this platform yet".to_string())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailArgs {
    pub id: String,
}

/// Capture a single source's thumbnail as base64 PNG. Called per-source in
/// parallel from the picker shim so the picker UI opens immediately and
/// thumbnails fade in as they arrive, rather than blocking the whole
/// enumeration call for 1-2 seconds on a many-window desktop.
#[tauri::command]
pub fn screen_share_thumbnail(args: ThumbnailArgs) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        macos::thumbnail_for_id(&args.id).ok_or_else(|| "thumbnail unavailable".to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = args;
        Err("thumbnails not implemented for this platform yet".to_string())
    }
}

// ---------------------------------------------------------------------------
// macOS backend
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod macos {
    use super::ScreenSource;

    use core::ffi::c_void;
    use std::ffi::CStr;

    // Minimal CoreGraphics FFI so we don't need an extra `core-graphics`
    // crate — these few symbols cover display + window enumeration and
    // avoid pulling in ~50 extra transitive deps.

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGGetActiveDisplayList(
            max_displays: u32,
            active_displays: *mut u32,
            display_count: *mut u32,
        ) -> i32;
        fn CGMainDisplayID() -> u32;
        fn CGDisplayPixelsWide(display: u32) -> usize;
        fn CGDisplayPixelsHigh(display: u32) -> usize;
        fn CGWindowListCopyWindowInfo(
            option: u32,
            relative_to_window: u32,
        ) -> *const c_void; // CFArrayRef
        fn CGDisplayCreateImage(display: u32) -> *const c_void; // CGImageRef
        fn CGWindowListCreateImage(
            screen_bounds: CGRect,
            list_option: u32,
            window_id: u32,
            image_option: u32,
        ) -> *const c_void;
        fn CGImageRelease(image: *const c_void);
        fn CGImageGetWidth(image: *const c_void) -> usize;
        fn CGImageGetHeight(image: *const c_void) -> usize;
    }

    #[link(name = "ImageIO", kind = "framework")]
    extern "C" {
        fn CGImageDestinationCreateWithData(
            data: *const c_void, // CFMutableDataRef
            uti: *const c_void,  // CFStringRef
            count: usize,
            options: *const c_void,
        ) -> *const c_void;
        fn CGImageDestinationAddImage(
            dest: *const c_void,
            image: *const c_void,
            properties: *const c_void,
        );
        fn CGImageDestinationFinalize(dest: *const c_void) -> bool;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const c_void);
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, idx: isize) -> *const c_void;
        fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
        fn CFStringGetCStringPtr(s: *const c_void, encoding: u32) -> *const i8;
        fn CFStringGetCString(
            s: *const c_void,
            buffer: *mut i8,
            buffer_size: isize,
            encoding: u32,
        ) -> bool;
        fn CFStringGetLength(s: *const c_void) -> isize;
        fn CFNumberGetValue(number: *const c_void, the_type: i32, value_ptr: *mut c_void) -> bool;
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const i8,
            encoding: u32,
        ) -> *const c_void;
        fn CFDataCreateMutable(alloc: *const c_void, capacity: isize) -> *const c_void;
        fn CFDataGetLength(data: *const c_void) -> isize;
        fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGPoint { x: f64, y: f64 }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGSize { width: f64, height: f64 }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGRect { origin: CGPoint, size: CGSize }

    const CG_RECT_NULL: CGRect = CGRect {
        origin: CGPoint { x: f64::INFINITY, y: f64::INFINITY },
        size: CGSize { width: 0.0, height: 0.0 },
    };
    // kCGWindowListOptionIncludingWindow.
    const K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW: u32 = 1 << 3;
    // kCGWindowImageBoundsIgnoreFraming | kCGWindowImageNominalResolution.
    const K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING: u32 = 1 << 0;
    const K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION: u32 = 1 << 4;

    const K_CFSTRING_ENCODING_UTF8: u32 = 0x08000100;
    const K_CFNUMBER_SINT64_TYPE: i32 = 4;
    // kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements.
    const K_CG_WINDOW_LIST_ON_SCREEN_ONLY: u32 = 1 << 0;
    const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;

    fn cfstr(s: &str) -> *const c_void {
        let c = std::ffi::CString::new(s).expect("cfstr contains NUL");
        unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), K_CFSTRING_ENCODING_UTF8) }
    }

    fn cfstring_to_string(cf: *const c_void) -> Option<String> {
        if cf.is_null() {
            return None;
        }
        unsafe {
            let ptr = CFStringGetCStringPtr(cf, K_CFSTRING_ENCODING_UTF8);
            if !ptr.is_null() {
                return CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string());
            }
            let len = CFStringGetLength(cf);
            // UTF-8 safety margin: 4 bytes per codepoint + NUL.
            let cap = (len as usize) * 4 + 1;
            let mut buf = vec![0i8; cap];
            if CFStringGetCString(cf, buf.as_mut_ptr(), cap as isize, K_CFSTRING_ENCODING_UTF8) {
                let c = CStr::from_ptr(buf.as_ptr());
                c.to_str().ok().map(|s| s.to_string())
            } else {
                None
            }
        }
    }

    fn cfnumber_to_u64(num: *const c_void) -> Option<u64> {
        if num.is_null() {
            return None;
        }
        let mut v: i64 = 0;
        unsafe {
            if CFNumberGetValue(num, K_CFNUMBER_SINT64_TYPE, &mut v as *mut _ as *mut c_void) {
                Some(v as u64)
            } else {
                None
            }
        }
    }

    /// Parse a `screen:<id>:0` / `window:<id>:0` source ID and capture its
    /// thumbnail as base64 PNG. Returns `None` if the ID is malformed or
    /// the underlying capture API returns a null/zero-size image.
    pub(super) fn thumbnail_for_id(id: &str) -> Option<String> {
        let mut parts = id.splitn(3, ':');
        let kind = parts.next()?;
        let num = parts.next()?.parse::<u32>().ok()?;
        let b64 = match kind {
            "screen" => screen_thumbnail_b64(num),
            "window" => window_thumbnail_b64(num),
            _ => return None,
        };
        if b64.is_empty() {
            None
        } else {
            Some(b64)
        }
    }

    pub(super) fn enumerate() -> Result<Vec<ScreenSource>, String> {
        let mut out = Vec::new();
        out.extend(enumerate_screens());
        out.extend(enumerate_windows());
        Ok(out)
    }

    /// Encode a CGImageRef as PNG bytes via ImageIO. Caller releases the
    /// image. Returns `None` on any ImageIO error so enumeration never
    /// fails because a single thumbnail couldn't be captured.
    fn cgimage_to_png_bytes(image: *const c_void) -> Option<Vec<u8>> {
        if image.is_null() {
            return None;
        }
        unsafe {
            let uti_key = cfstr("public.png");
            let data = CFDataCreateMutable(std::ptr::null(), 0);
            if data.is_null() {
                CFRelease(uti_key);
                return None;
            }
            let dest = CGImageDestinationCreateWithData(data, uti_key, 1, std::ptr::null());
            if dest.is_null() {
                CFRelease(uti_key);
                CFRelease(data);
                return None;
            }
            CGImageDestinationAddImage(dest, image, std::ptr::null());
            let ok = CGImageDestinationFinalize(dest);
            CFRelease(dest);
            CFRelease(uti_key);
            if !ok {
                CFRelease(data);
                return None;
            }
            let len = CFDataGetLength(data) as usize;
            let ptr = CFDataGetBytePtr(data);
            let bytes = std::slice::from_raw_parts(ptr, len).to_vec();
            CFRelease(data);
            Some(bytes)
        }
    }

    fn screen_thumbnail_b64(display_id: u32) -> String {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        unsafe {
            let image = CGDisplayCreateImage(display_id);
            if image.is_null() {
                return String::new();
            }
            let png = cgimage_to_png_bytes(image);
            CGImageRelease(image);
            png.map(|b| STANDARD.encode(b)).unwrap_or_default()
        }
    }

    fn window_thumbnail_b64(window_id: u32) -> String {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        unsafe {
            let opts = K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING
                | K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION;
            let image = CGWindowListCreateImage(
                CG_RECT_NULL,
                K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW,
                window_id,
                opts,
            );
            if image.is_null() {
                return String::new();
            }
            if CGImageGetWidth(image) == 0 || CGImageGetHeight(image) == 0 {
                CGImageRelease(image);
                return String::new();
            }
            let png = cgimage_to_png_bytes(image);
            CGImageRelease(image);
            png.map(|b| STANDARD.encode(b)).unwrap_or_default()
        }
    }

    fn enumerate_screens() -> Vec<ScreenSource> {
        let mut ids = [0u32; 32];
        let mut count: u32 = 0;
        let err = unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
        if err != 0 {
            log::warn!("[screen-share] CGGetActiveDisplayList error={err}");
            return Vec::new();
        }
        let main = unsafe { CGMainDisplayID() };
        ids.iter()
            .take(count as usize)
            .enumerate()
            .map(|(idx, &display_id)| {
                let w = unsafe { CGDisplayPixelsWide(display_id) };
                let h = unsafe { CGDisplayPixelsHigh(display_id) };
                let is_main = display_id == main;
                let name = if is_main {
                    format!("Main Screen ({}×{})", w, h)
                } else {
                    format!("Display {} ({}×{})", idx + 1, w, h)
                };
                ScreenSource {
                    id: format!("screen:{}:0", display_id),
                    kind: "screen".to_string(),
                    name,
                    app_name: None,
                    // Thumbnails are now lazy-fetched by the shim via
                    // `screen_share_thumbnail` in parallel with the
                    // picker render, so enumeration stays fast.
                    thumbnail_png_base64: String::new(),
                }
            })
            .collect()
    }

    fn enumerate_windows() -> Vec<ScreenSource> {
        let opts = K_CG_WINDOW_LIST_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
        let array = unsafe { CGWindowListCopyWindowInfo(opts, 0) };
        if array.is_null() {
            log::warn!("[screen-share] CGWindowListCopyWindowInfo returned null");
            return Vec::new();
        }
        let key_window_number = cfstr("kCGWindowNumber");
        let key_window_name = cfstr("kCGWindowName");
        let key_owner_name = cfstr("kCGWindowOwnerName");
        let key_bounds = cfstr("kCGWindowBounds");
        let key_layer = cfstr("kCGWindowLayer");

        let count = unsafe { CFArrayGetCount(array) };
        let mut out: Vec<ScreenSource> = Vec::new();
        for i in 0..count {
            let dict = unsafe { CFArrayGetValueAtIndex(array, i) };
            if dict.is_null() {
                continue;
            }
            let number_cf = unsafe { CFDictionaryGetValue(dict, key_window_number) };
            let layer_cf = unsafe { CFDictionaryGetValue(dict, key_layer) };
            let window_id = match cfnumber_to_u64(number_cf) {
                Some(v) => v,
                None => continue,
            };
            // Skip menu bar / dock / system chrome (layer != 0 → non-normal
            // window). Normal app windows live at layer 0.
            let layer = cfnumber_to_u64(layer_cf).unwrap_or(0);
            if layer != 0 {
                continue;
            }
            // Skip microscopic windows (tooltips, hidden panels).
            if let Some(bounds_dict) = unsafe {
                CFDictionaryGetValue(dict, key_bounds).as_ref()
            } {
                // kCGWindowBounds is actually a CFDictionary with Width/Height
                // keys. Cheap filter: if the dict has a "Width" key and it's
                // < 50, skip. Implementing full parse isn't worth it for the
                // MVP; Chromium renders a scrollable picker anyway.
                let _ = bounds_dict;
            }
            let title = unsafe { CFDictionaryGetValue(dict, key_window_name) };
            let owner = unsafe { CFDictionaryGetValue(dict, key_owner_name) };
            let title_str = cfstring_to_string(title).unwrap_or_default();
            let owner_str = cfstring_to_string(owner).unwrap_or_default();
            // Windows with no title are usually uninteresting (background
            // helpers). Skip unless owner is informative and the window is
            // the owner's only one — for MVP, simpler to just drop them.
            if title_str.is_empty() {
                continue;
            }
            let name = if owner_str.is_empty() {
                title_str.clone()
            } else {
                format!("{} — {}", owner_str, title_str)
            };
            out.push(ScreenSource {
                id: format!("window:{}:0", window_id),
                kind: "window".to_string(),
                name,
                app_name: if owner_str.is_empty() {
                    None
                } else {
                    Some(owner_str)
                },
                thumbnail_png_base64: String::new(),
            });
        }
        unsafe {
            CFRelease(key_window_number);
            CFRelease(key_window_name);
            CFRelease(key_owner_name);
            CFRelease(key_bounds);
            CFRelease(key_layer);
            CFRelease(array);
        }
        out
    }
}
