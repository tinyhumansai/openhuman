//! Screen-capture source enumeration + picker orchestration for #713 / #812.
//!
//! Background (see issue #713 plan): embedded webviews (Meet, Slack Huddles,
//! Discord, Zoom) run under the CEF Alloy runtime, which does not link
//! Chromium's built-in `DesktopMediaPicker`. When the page calls
//! `navigator.mediaDevices.getDisplayMedia`, Chromium falls back to
//! auto-selecting the primary display — the user never sees a picker and
//! their whole screen streams.
//!
//! Our `OnRequestMediaAccessPermission` callback in tauri-cef grants the
//! `DESKTOP_VIDEO_CAPTURE` bit unconditionally. Stage 0 PoC proved that when
//! the page calls `getUserMedia` with a hand-crafted
//! `{ mandatory: { chromeMediaSource: 'desktop', chromeMediaSourceId: '<id>' } }`
//! constraint, Chromium honours the ID and opens a real capture device —
//! even though this constraint shape is normally extension-only.
//!
//! # Session gating (#812 Stage A)
//!
//! The first landing of this module exposed `screen_share_list_sources` and
//! `screen_share_thumbnail` directly on the recipe-webview allowlist. That
//! let any script running inside the embedded site (page JS, compromised
//! third-party CDN) silently enumerate every open window title + live
//! thumbnail with no picker interaction and no user gesture. CodeRabbit /
//! graycyrus flagged this as a blocker on PR #809 (issue #812).
//!
//! The module now forces callers through a short-lived session:
//!   * `screen_share_begin_session` — requires a live user gesture
//!     (`navigator.userActivation.isActive`), an account-scoped webview
//!     label (`acct_*`), and is rate-limited to 10 calls per account per
//!     60s. Returns a random 128-bit token + the enumerated sources in
//!     one round-trip.
//!   * `screen_share_thumbnail` — requires a token whose session is still
//!     alive and whose `allowed_ids` set contains the requested ID.
//!   * `screen_share_finalize_session` — removes the session. Called by
//!     the shim on Share or Cancel.
//!
//! Sessions auto-expire after 30s. A new `begin_session` for the same
//! account replaces any in-flight session (prevents the stacked-overlay
//! case from graycyrus refactor note #6).
//!
//! The picker UI itself is injected directly into each child webview's
//! DOM by `webview_accounts/runtime.js` (see the `showInPagePicker` flow
//! there), which is why we only need IPCs for enumeration + thumbnail
//! capture and no picker-modal orchestration RPCs on the host side.
//!
//! macOS-first: other platforms stub out until the flow is proven end-
//! to-end.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{Runtime, State, Webview};

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
    /// PNG thumbnail base64-encoded. Always empty from enumeration — the
    /// shim lazy-fetches via `screen_share_thumbnail` so the picker UI opens
    /// instantly.
    #[serde(default)]
    pub thumbnail_png_base64: String,
}

// ---------------------------------------------------------------------------
// Parser (platform-agnostic, unit-testable)
// ---------------------------------------------------------------------------

/// What kind of source a parsed DesktopMediaID-format string describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceKind {
    Screen,
    Window,
}

/// Parse a `screen:<u32>:0` / `window:<u32>:0` source ID into
/// `(kind, numeric id)`. Returns `None` if the prefix is unknown, the
/// numeric segment doesn't fit in a `u32`, or the shape otherwise doesn't
/// match what the enumerator emits. Pure logic so it can be unit-tested
/// without touching platform APIs; macOS callers use it before dispatching
/// to the capture backend.
pub(crate) fn parse_source_id(id: &str) -> Option<(SourceKind, u32)> {
    let mut parts = id.splitn(3, ':');
    let kind = match parts.next()? {
        "screen" => SourceKind::Screen,
        "window" => SourceKind::Window,
        _ => return None,
    };
    let num = parts.next()?.parse::<u32>().ok()?;
    Some((kind, num))
}

// ---------------------------------------------------------------------------
// Session state (#812 Stage A)
// ---------------------------------------------------------------------------

/// Short TTL prevents stale tokens from being replayable. 30s is long enough
/// for the slowest picker flow (enumerate → thumbs load → user chooses)
/// observed in manual testing, short enough that a leaked token via console
/// can't be reused later in the day.
const SESSION_TTL: Duration = Duration::from_secs(30);
/// Token bucket parameters. 10 attempts per 60s per account means a human
/// mashing the Present-Now button can't get throttled; an automated
/// enumeration loop hits the wall quickly.
const RATE_LIMIT_MAX: usize = 10;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);
/// 128-bit token. Seeded from OS time + atomic counter + thread id —
/// deliberately no new dependency. Entropy is overkill for a 30s session:
/// the attacker would need to guess the token AND the account-id AND the
/// allowed-id set inside the TTL window.
const TOKEN_BYTES: usize = 16;

static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(1);

fn generate_token() -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let counter = TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tid = thread_id_hash();
    let mut buf = [0u8; TOKEN_BYTES];
    // Interleave the three sources across the 16 bytes so no single
    // predictable input (wall clock, counter) dominates the prefix.
    buf[0..8].copy_from_slice(&(now as u64).to_le_bytes());
    buf[8..16].copy_from_slice(&counter.to_le_bytes());
    for (i, b) in buf.iter_mut().enumerate() {
        *b ^= tid.rotate_left((i as u32) * 3);
    }
    URL_SAFE_NO_PAD.encode(buf)
}

fn thread_id_hash() -> u8 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::thread::current().id().hash(&mut h);
    h.finish() as u8
}

#[derive(Debug)]
struct Session {
    account_id: String,
    allowed_ids: HashSet<String>,
    expires_at: Instant,
}

#[derive(Default)]
pub struct ScreenShareState {
    /// token → Session
    sessions: Mutex<HashMap<String, Session>>,
    /// account_id → rolling window of begin-session timestamps for rate limit
    rate: Mutex<HashMap<String, VecDeque<Instant>>>,
    /// account_id → current active token (so we can evict on replace)
    active: Mutex<HashMap<String, String>>,
}

impl ScreenShareState {
    pub fn new() -> Self {
        Self::default()
    }
}

fn purge_expired(sessions: &mut HashMap<String, Session>, active: &mut HashMap<String, String>) {
    let now = Instant::now();
    let expired_tokens: Vec<String> = sessions
        .iter()
        .filter_map(|(t, s)| {
            if s.expires_at <= now {
                Some(t.clone())
            } else {
                None
            }
        })
        .collect();
    for t in expired_tokens {
        if let Some(sess) = sessions.remove(&t) {
            if active.get(&sess.account_id).map(|x| x.as_str()) == Some(t.as_str()) {
                active.remove(&sess.account_id);
            }
        }
    }
}

fn check_and_record_rate(rate: &mut HashMap<String, VecDeque<Instant>>, account_id: &str) -> bool {
    let now = Instant::now();
    let window = rate.entry(account_id.to_string()).or_default();
    while let Some(&front) = window.front() {
        if now.duration_since(front) > RATE_LIMIT_WINDOW {
            window.pop_front();
        } else {
            break;
        }
    }
    if window.len() >= RATE_LIMIT_MAX {
        return false;
    }
    window.push_back(now);
    true
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginSessionArgs {
    pub account_id: String,
    pub origin: String,
    /// Frontend-reported `navigator.userActivation.isActive`. True only while
    /// the call stack originates from a real user gesture (click, key, touch)
    /// within the page's activation grace period. False for timers, async
    /// continuations, or drive-by enumeration attempts.
    pub has_user_activation: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginSessionResult {
    pub token: String,
    pub sources: Vec<ScreenSource>,
}

/// Open a short-lived session that gates subsequent `screen_share_thumbnail`
/// calls. The shim must call this before showing the picker UI; any page JS
/// attempting the same call outside a user gesture is rejected.
#[tauri::command]
pub fn screen_share_begin_session<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, ScreenShareState>,
    args: BeginSessionArgs,
) -> Result<BeginSessionResult, String> {
    let caller_label = webview.label().to_string();
    log::debug!(
        "[screen-share] begin_session caller_label={} account_id={} origin={} activation={}",
        caller_label,
        args.account_id,
        args.origin,
        args.has_user_activation
    );

    // Gate 1: caller must be an account webview. `acct_*` is the label shape
    // produced by `webview_accounts::label_for()`. Main/overlay windows and
    // any other Tauri webview fail here.
    if !caller_label.starts_with("acct_") {
        log::warn!(
            "[screen-share] begin_session rejected: caller_label={} is not an account webview",
            caller_label
        );
        return Err("unauthorized caller".to_string());
    }

    // Gate 2: must be inside a user gesture. Frontend reads
    // `navigator.userActivation.isActive` which is true only during the
    // direct call stack of a click / key / touch handler.
    if !args.has_user_activation {
        log::warn!(
            "[screen-share] begin_session rejected: no user activation for account_id={}",
            args.account_id
        );
        return Err("user activation required".to_string());
    }

    // Housekeeping before checking rate / active state.
    {
        let mut sessions = state
            .sessions
            .lock()
            .expect("screen_share.sessions poisoned");
        let mut active = state.active.lock().expect("screen_share.active poisoned");
        purge_expired(&mut sessions, &mut active);
    }

    // Gate 3: rate limit per account.
    {
        let mut rate = state.rate.lock().expect("screen_share.rate poisoned");
        if !check_and_record_rate(&mut rate, &args.account_id) {
            log::warn!(
                "[screen-share] begin_session rate-limited account_id={} (>{} within {:?})",
                args.account_id,
                RATE_LIMIT_MAX,
                RATE_LIMIT_WINDOW
            );
            return Err("rate-limited".to_string());
        }
    }

    // Enumerate sources and build the session.
    let sources = enumerate_sources()?;
    let allowed_ids: HashSet<String> = sources.iter().map(|s| s.id.clone()).collect();
    let token = generate_token();
    let token_display = token_prefix(&token);

    {
        let mut sessions = state
            .sessions
            .lock()
            .expect("screen_share.sessions poisoned");
        let mut active = state.active.lock().expect("screen_share.active poisoned");

        // Replace any in-flight session for this account — prevents stacked
        // pickers if getDisplayMedia is called twice before the first
        // resolves (graycyrus refactor #6).
        if let Some(prev) = active.remove(&args.account_id) {
            sessions.remove(&prev);
            log::debug!(
                "[screen-share] begin_session replacing prev session token={}…",
                token_prefix(&prev)
            );
        }

        sessions.insert(
            token.clone(),
            Session {
                account_id: args.account_id.clone(),
                allowed_ids,
                expires_at: Instant::now() + SESSION_TTL,
            },
        );
        active.insert(args.account_id.clone(), token.clone());
    }

    log::info!(
        "[screen-share] begin_session opened token={}… account_id={} sources={}",
        token_display,
        args.account_id,
        sources.len()
    );

    Ok(BeginSessionResult { token, sources })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailArgs {
    pub token: String,
    pub id: String,
}

/// Capture one source's thumbnail as base64 PNG. Gated behind the session
/// token: only IDs the session was issued for (i.e. shown in the picker)
/// can be thumbnailed, so a valid token can't be abused to snapshot
/// arbitrary windows.
#[tauri::command]
pub fn screen_share_thumbnail<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, ScreenShareState>,
    args: ThumbnailArgs,
) -> Result<String, String> {
    let caller_label = webview.label().to_string();
    log::debug!(
        "[screen-share] thumbnail caller_label={} id={} token={}…",
        caller_label,
        args.id,
        token_prefix(&args.token)
    );

    if !caller_label.starts_with("acct_") {
        log::warn!(
            "[screen-share] thumbnail rejected: caller_label={} is not an account webview",
            caller_label
        );
        return Err("unauthorized caller".to_string());
    }

    // Validate the session is alive and knows about this ID.
    {
        let mut sessions = state
            .sessions
            .lock()
            .expect("screen_share.sessions poisoned");
        let mut active = state.active.lock().expect("screen_share.active poisoned");
        purge_expired(&mut sessions, &mut active);

        let session = sessions.get(&args.token).ok_or_else(|| {
            log::warn!(
                "[screen-share] thumbnail rejected: unknown/expired token={}…",
                token_prefix(&args.token)
            );
            "invalid or expired token".to_string()
        })?;
        if !session.allowed_ids.contains(&args.id) {
            log::warn!(
                "[screen-share] thumbnail rejected: id={} not in session's allowed set (token={}…)",
                args.id,
                token_prefix(&args.token)
            );
            return Err("id not in session".to_string());
        }
    }

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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizeSessionArgs {
    pub token: String,
    #[serde(default)]
    pub picked_id: Option<String>,
}

/// Called by the shim on Share or Cancel. Removes the session. Safe to call
/// with an unknown/expired token — the call is a no-op then. Not gated on
/// caller label because the only effect is cleanup of a token the caller
/// already possesses.
#[tauri::command]
pub fn screen_share_finalize_session(
    state: State<'_, ScreenShareState>,
    args: FinalizeSessionArgs,
) -> Result<(), String> {
    let token_display = token_prefix(&args.token);
    let mut sessions = state
        .sessions
        .lock()
        .expect("screen_share.sessions poisoned");
    let mut active = state.active.lock().expect("screen_share.active poisoned");
    purge_expired(&mut sessions, &mut active);

    if let Some(session) = sessions.remove(&args.token) {
        if active.get(&session.account_id).map(|x| x.as_str()) == Some(args.token.as_str()) {
            active.remove(&session.account_id);
        }
        log::info!(
            "[screen-share] finalize_session token={}… account_id={} picked={}",
            token_display,
            session.account_id,
            args.picked_id.as_deref().unwrap_or("<cancelled>")
        );
    } else {
        log::debug!(
            "[screen-share] finalize_session ignored (unknown token={}…)",
            token_display
        );
    }
    Ok(())
}

fn token_prefix(token: &str) -> String {
    token.chars().take(8).collect()
}

fn enumerate_sources() -> Result<Vec<ScreenSource>, String> {
    #[cfg(target_os = "macos")]
    {
        macos::enumerate().map_err(|e| format!("enumerate failed: {e}"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("screen-share picker not implemented for this platform yet".to_string())
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
        fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> *const c_void; // CFArrayRef
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
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGSize {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    const CG_RECT_NULL: CGRect = CGRect {
        origin: CGPoint {
            x: f64::INFINITY,
            y: f64::INFINITY,
        },
        size: CGSize {
            width: 0.0,
            height: 0.0,
        },
    };
    // kCGWindowListOptionIncludingWindow (= 8).
    const K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW: u32 = 1 << 3;
    // kCGWindowImageBoundsIgnoreFraming (= 1) | kCGWindowImageNominalResolution (= 16).
    const K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING: u32 = 1 << 0;
    const K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION: u32 = 1 << 4;

    const K_CFSTRING_ENCODING_UTF8: u32 = 0x08000100;
    const K_CFNUMBER_SINT64_TYPE: i32 = 4;
    // kCGWindowListOptionOnScreenOnly (= 1) | kCGWindowListExcludeDesktopElements (= 16).
    const K_CG_WINDOW_LIST_ON_SCREEN_ONLY: u32 = 1 << 0;
    const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;

    /// Below this pixel count on either axis we treat a captured window
    /// image as TCC-denied rather than real content. macOS 15 Sequoia
    /// returns a valid 1×1 transparent CGImage when Screen Recording is
    /// not granted (instead of the pre-Sequoia null return), and the old
    /// empty-check alone let that through (see PR #809 review).
    const MIN_USABLE_DIMENSION: usize = 4;

    /// Allocate a CoreFoundation string. Returns `None` if the input
    /// contains an interior NUL byte (CString rejects those). Callers
    /// check the return rather than `expect()`ing, because unwinding
    /// through a C frame is undefined behavior.
    fn cfstr(s: &str) -> Option<*const c_void> {
        let c = std::ffi::CString::new(s).ok()?;
        let ptr = unsafe {
            CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), K_CFSTRING_ENCODING_UTF8)
        };
        if ptr.is_null() {
            None
        } else {
            Some(ptr)
        }
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

    pub(super) fn thumbnail_for_id(id: &str) -> Option<String> {
        let (kind, num) = super::parse_source_id(id)?;
        let b64 = match kind {
            super::SourceKind::Screen => screen_thumbnail_b64(num),
            super::SourceKind::Window => window_thumbnail_b64(num),
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

    fn cgimage_to_png_bytes(image: *const c_void) -> Option<Vec<u8>> {
        if image.is_null() {
            return None;
        }
        let uti_key = cfstr("public.png")?;
        unsafe {
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
            let w = CGImageGetWidth(image);
            let h = CGImageGetHeight(image);
            if w < MIN_USABLE_DIMENSION || h < MIN_USABLE_DIMENSION {
                log::warn!(
                    "[screen-share] screen_thumbnail display_id={} returned {}×{} (likely TCC not granted)",
                    display_id,
                    w,
                    h
                );
                CGImageRelease(image);
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
            let opts =
                K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING | K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION;
            let image = CGWindowListCreateImage(
                CG_RECT_NULL,
                K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW,
                window_id,
                opts,
            );
            if image.is_null() {
                return String::new();
            }
            let w = CGImageGetWidth(image);
            let h = CGImageGetHeight(image);
            if w < MIN_USABLE_DIMENSION || h < MIN_USABLE_DIMENSION {
                log::warn!(
                    "[screen-share] window_thumbnail window_id={} returned {}×{} (likely TCC not granted or Sequoia privacy policy)",
                    window_id,
                    w,
                    h
                );
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

        // cfstr can fail (interior NUL — never happens for these literals
        // but stay defensive); bail cleanly if so.
        let Some(key_window_number) = cfstr("kCGWindowNumber") else {
            unsafe { CFRelease(array) };
            return Vec::new();
        };
        let Some(key_window_name) = cfstr("kCGWindowName") else {
            unsafe {
                CFRelease(key_window_number);
                CFRelease(array)
            };
            return Vec::new();
        };
        let Some(key_owner_name) = cfstr("kCGWindowOwnerName") else {
            unsafe {
                CFRelease(key_window_number);
                CFRelease(key_window_name);
                CFRelease(array);
            }
            return Vec::new();
        };
        let Some(key_layer) = cfstr("kCGWindowLayer") else {
            unsafe {
                CFRelease(key_window_number);
                CFRelease(key_window_name);
                CFRelease(key_owner_name);
                CFRelease(array);
            }
            return Vec::new();
        };

        let count = unsafe { CFArrayGetCount(array) };
        let mut out: Vec<ScreenSource> = Vec::new();
        for i in 0..count {
            let dict = unsafe { CFArrayGetValueAtIndex(array, i) };
            if dict.is_null() {
                continue;
            }
            let number_cf = unsafe { CFDictionaryGetValue(dict, key_window_number) };
            let layer_cf = unsafe { CFDictionaryGetValue(dict, key_layer) };
            let window_id_u64 = match cfnumber_to_u64(number_cf) {
                Some(v) => v,
                None => continue,
            };
            // `CGWindowID` is `uint32_t` upstream, but `cfnumber_to_u64`
            // returns 64-bit (we read the CFNumber as SInt64 for sign
            // safety). Values should never exceed `u32::MAX` in practice,
            // but a silent cast would round-trip through `format!` and
            // then fail parse_source_id — the user would see a source in
            // the picker with a permanent grey placeholder. Skip loudly.
            let window_id = match u32::try_from(window_id_u64) {
                Ok(v) => v,
                Err(_) => {
                    log::warn!(
                        "[screen-share] window_id {} overflows u32, skipping",
                        window_id_u64
                    );
                    continue;
                }
            };
            // Skip menu bar / dock / system chrome (layer != 0 → non-normal
            // window). Normal app windows live at layer 0.
            let layer = cfnumber_to_u64(layer_cf).unwrap_or(0);
            if layer != 0 {
                continue;
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
            CFRelease(key_layer);
            CFRelease(array);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse_source_id tests (platform-agnostic) ----

    #[test]
    fn parses_screen_id() {
        assert_eq!(parse_source_id("screen:1:0"), Some((SourceKind::Screen, 1)));
        assert_eq!(
            parse_source_id("screen:69734208:0"),
            Some((SourceKind::Screen, 69734208))
        );
    }

    #[test]
    fn parses_window_id() {
        assert_eq!(
            parse_source_id("window:42:0"),
            Some((SourceKind::Window, 42))
        );
    }

    #[test]
    fn trailing_segment_ignored() {
        assert_eq!(
            parse_source_id("screen:1:extra:stuff"),
            Some((SourceKind::Screen, 1))
        );
    }

    #[test]
    fn rejects_unknown_prefix() {
        assert_eq!(parse_source_id("tab:1:0"), None);
        assert_eq!(parse_source_id("browser:1:0"), None);
        assert_eq!(parse_source_id(""), None);
    }

    #[test]
    fn rejects_missing_numeric() {
        assert_eq!(parse_source_id("screen::0"), None);
        assert_eq!(parse_source_id("screen:"), None);
        assert_eq!(parse_source_id("screen"), None);
    }

    #[test]
    fn rejects_non_numeric_id() {
        assert_eq!(parse_source_id("screen:abc:0"), None);
        assert_eq!(parse_source_id("window:0x1:0"), None);
    }

    #[test]
    fn rejects_overflowing_id() {
        assert_eq!(parse_source_id("screen:4294967296:0"), None);
        assert_eq!(parse_source_id("screen:-1:0"), None);
    }

    #[test]
    fn list_source_roundtrip() {
        assert!(parse_source_id("screen:1:0").is_some());
        assert!(parse_source_id("window:12345:0").is_some());
    }

    // ---- Session / rate-limit tests (pure logic, no platform APIs) ----

    fn insert_test_session(
        state: &ScreenShareState,
        token: &str,
        account_id: &str,
        ttl: Duration,
        ids: &[&str],
    ) {
        let mut sessions = state.sessions.lock().unwrap();
        let mut active = state.active.lock().unwrap();
        sessions.insert(
            token.to_string(),
            Session {
                account_id: account_id.to_string(),
                allowed_ids: ids.iter().map(|s| s.to_string()).collect(),
                expires_at: Instant::now() + ttl,
            },
        );
        active.insert(account_id.to_string(), token.to_string());
    }

    #[test]
    fn purge_expired_removes_stale_sessions() {
        let state = ScreenShareState::new();
        insert_test_session(
            &state,
            "tok-expired",
            "acct1",
            Duration::from_millis(0),
            &[],
        );
        // Sleep a blink so `expires_at <= now` is definitely true.
        std::thread::sleep(Duration::from_millis(5));
        insert_test_session(&state, "tok-live", "acct2", Duration::from_secs(10), &[]);

        {
            let mut s = state.sessions.lock().unwrap();
            let mut a = state.active.lock().unwrap();
            purge_expired(&mut s, &mut a);
        }

        let sessions = state.sessions.lock().unwrap();
        assert!(!sessions.contains_key("tok-expired"));
        assert!(sessions.contains_key("tok-live"));
        let active = state.active.lock().unwrap();
        assert!(!active.contains_key("acct1"));
        assert_eq!(active.get("acct2").map(|s| s.as_str()), Some("tok-live"));
    }

    #[test]
    fn rate_limit_blocks_11th_call_in_window() {
        let mut rate: HashMap<String, VecDeque<Instant>> = HashMap::new();
        for _ in 0..RATE_LIMIT_MAX {
            assert!(check_and_record_rate(&mut rate, "acct-x"));
        }
        // 11th call must fail.
        assert!(!check_and_record_rate(&mut rate, "acct-x"));
    }

    #[test]
    fn rate_limit_scoped_per_account() {
        let mut rate: HashMap<String, VecDeque<Instant>> = HashMap::new();
        for _ in 0..RATE_LIMIT_MAX {
            check_and_record_rate(&mut rate, "acct-a");
        }
        // Different account still has full budget.
        assert!(check_and_record_rate(&mut rate, "acct-b"));
    }

    #[test]
    fn generate_token_is_url_safe_and_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b);
        // URL-safe base64, no-pad, 16 bytes → 22 chars.
        assert_eq!(a.len(), 22);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn token_prefix_truncates() {
        assert_eq!(token_prefix("0123456789abcdef"), "01234567");
        assert_eq!(token_prefix("ab"), "ab");
    }

    // NOTE: full command-level tests (screen_share_begin_session etc.)
    // would need a `tauri::Webview` mock, which the stable Tauri API
    // doesn't expose. Gate + rate-limit logic is covered above; the
    // command glue around it is thin enough to verify via live run.
}
