//! Native macOS NSPanel + WKWebView host for the notch activity indicator.
//!
//! A transparent, click-through floating panel anchored to the top-centre of
//! the primary screen. On MacBook Pros with a physical notch the pill visually
//! emerges from the notch; on older Macs it acts as a top-centre floating HUD.
//!
//! Architecture mirrors `mascot_native_window` — a native NSPanel avoids the
//! CEF transparency limitation (vendored tauri-cef cannot render transparent
//! windowed-mode browsers; only off-screen rendering supports transparency,
//! which the runtime does not enable). The WKWebView loads the same Vite entry
//! point at `?window=notch` so the React tree can branch in `main.tsx`.
//!
//! IPC strategy: no Tauri IPC bridge. The panel polls
//! `OPENHUMAN_CORE_RPC_URL` (set by `CoreProcessHandle` once the embedded
//! server is ready) and injects it via `evaluateJavaScript` so the React app
//! can open a Socket.IO connection to receive live voice and agent events.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSPanel, NSScreen, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSNumber, NSPoint, NSRect, NSSize, NSString, NSTimer, NSURLRequest, NSURL};
use objc2_web_kit::{WKWebView, WKWebViewConfiguration};
use tauri::{AppHandle, Manager};

use crate::AppRuntime;

/// Logical width of the notch panel. Wide enough to display voice/action text.
const PANEL_WIDTH: f64 = 380.0;
/// Logical height — covers the menu-bar / notch depth with headroom for the pill.
const PANEL_HEIGHT: f64 = 54.0;
/// URL-inject timer interval in seconds.
const INJECT_POLL_SECONDS: f64 = 1.0;
/// Ticks to wait before the first inject attempt (page-load delay).
const PAGE_LOAD_TICKS: u32 = 2;

struct NotchPanel {
    panel: Retained<NSPanel>,
    #[allow(dead_code)]
    webview: Retained<WKWebView>,
    inject_timer: Retained<NSTimer>,
}

impl NotchPanel {
    fn order_out(&self) {
        self.inject_timer.invalidate();
        self.panel.orderOut(None);
    }
}

thread_local! {
    /// Accessed only from the main thread. NSPanel/WKWebView are not Send/Sync
    /// so a thread-local is the simplest safe storage.
    static NOTCH: RefCell<Option<NotchPanel>> = const { RefCell::new(None) };
}

pub(crate) fn is_open() -> bool {
    NOTCH.with(|cell| cell.borrow().is_some())
}

pub(crate) fn hide() {
    NOTCH.with(|cell| {
        if let Some(existing) = cell.borrow_mut().take() {
            log::info!("[notch-window] dropping panel");
            existing.order_out();
        }
    });
}

pub(crate) fn show(app: &AppHandle<AppRuntime>) -> Result<(), String> {
    if NOTCH.with(|cell| cell.borrow().is_some()) {
        log::debug!("[notch-window] already open");
        return Ok(());
    }

    let mtm = MainThreadMarker::new()
        .ok_or_else(|| "notch_window::show called off the main thread".to_string())?;

    let source = resolve_page_source(app)?;
    // Log only the source *kind* — bundled paths contain `/Users/<login>/…`
    // (PII), so never log the absolute resource paths.
    log::info!(
        "[notch-window] loading source_kind={}",
        match &source {
            PageSource::Dev { .. } => "dev",
            PageSource::Bundled { .. } => "bundled",
        }
    );

    let frame = top_center_frame(mtm);
    log::debug!(
        "[notch-window] frame origin=({:.0},{:.0}) size=({:.0},{:.0})",
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height
    );

    let panel = unsafe { build_panel(mtm, frame) };
    let webview = unsafe { build_webview(mtm, &panel, &source) };

    panel.orderFrontRegardless();

    let inject_timer = unsafe { spawn_inject_timer(webview.clone()) };

    NOTCH.with(|cell| {
        *cell.borrow_mut() = Some(NotchPanel {
            panel,
            webview,
            inject_timer,
        });
    });
    log::info!("[notch-window] panel shown at top-center");
    Ok(())
}

// ── Page source ───────────────────────────────────────────────────────────────

#[derive(Debug)]
enum PageSource {
    Dev { url: String },
    Bundled { index_html: PathBuf, root: PathBuf },
}

fn resolve_page_source(app: &AppHandle<AppRuntime>) -> Result<PageSource, String> {
    if let Some(mut url) = app.config().build.dev_url.as_ref().cloned() {
        let query = url
            .query()
            .map(|q| format!("{q}&window=notch"))
            .unwrap_or_else(|| "window=notch".into());
        url.set_query(Some(&query));
        return Ok(PageSource::Dev {
            url: url.to_string(),
        });
    }

    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|e| format!("resolve resource_dir: {e}"))?;
    for candidate in [
        resource_dir.join("index.html"),
        resource_dir.join("dist").join("index.html"),
    ] {
        if candidate.is_file() {
            let root = candidate
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| resource_dir.clone());
            return Ok(PageSource::Bundled {
                index_html: candidate,
                root,
            });
        }
    }
    Err("notch bundled index.html not found under the app resource dir".to_string())
}

// ── Frame geometry ────────────────────────────────────────────────────────────

fn primary_screen_frame(mtm: MainThreadMarker) -> NSRect {
    let screens = NSScreen::screens(mtm);
    if let Some(primary) = screens.firstObject() {
        return primary.frame();
    }
    log::warn!("[notch-window] NSScreen::screens returned empty — falling back to 1440×900");
    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0))
}

/// Centre the panel horizontally at the very top of the primary screen.
///
/// AppKit uses a bottom-left origin, so:
///   top-y  = screen.origin.y + screen.height − PANEL_HEIGHT
///   center-x = screen.origin.x + (screen.width − PANEL_WIDTH) / 2
fn top_center_frame(mtm: MainThreadMarker) -> NSRect {
    let screen = primary_screen_frame(mtm);
    let x = screen.origin.x + (screen.size.width - PANEL_WIDTH) / 2.0;
    let y = screen.origin.y + screen.size.height - PANEL_HEIGHT;
    NSRect::new(NSPoint::new(x, y), NSSize::new(PANEL_WIDTH, PANEL_HEIGHT))
}

// ── NSPanel construction ──────────────────────────────────────────────────────

unsafe fn build_panel(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSPanel> {
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let panel: Retained<NSPanel> = unsafe {
        let allocated = NSPanel::alloc(mtm);
        msg_send![
            allocated,
            initWithContentRect: frame,
            styleMask: style,
            backing: NSBackingStoreType::Buffered,
            defer: false,
        ]
    };

    unsafe {
        panel.setOpaque(false);
        let clear = NSColor::clearColor();
        panel.setBackgroundColor(Some(&clear));
        panel.setHasShadow(false);

        // Float above the menu bar. NSStatusWindowLevel = 25, which sits above
        // NSMainMenuWindowLevel = 24. Same recipe used by the mascot panel and
        // the `configure_overlay_window_macos` helper.
        panel.setLevel(25);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Transient
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        panel.setFloatingPanel(true);
        panel.setHidesOnDeactivate(false);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setWorksWhenModal(true);

        // Fully click-through: the panel never steals mouse events. Menu-bar
        // items remain clickable through the transparent regions.
        panel.setIgnoresMouseEvents(true);

        let _: () = msg_send![&*panel, setExcludedFromWindowsMenu: true];
    }

    panel
}

// ── WKWebView construction ────────────────────────────────────────────────────

unsafe fn build_webview(
    mtm: MainThreadMarker,
    panel: &NSPanel,
    source: &PageSource,
) -> Retained<WKWebView> {
    let config: Retained<WKWebViewConfiguration> = unsafe {
        let alloc = WKWebViewConfiguration::alloc(mtm);
        msg_send![alloc, init]
    };

    let frame = NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(PANEL_WIDTH, PANEL_HEIGHT),
    );
    let webview: Retained<WKWebView> =
        unsafe { WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config) };

    unsafe {
        // Disable WKWebView's own background so CSS `background: transparent` works.
        // There is no public API for this on macOS — KVC against the private
        // `drawsBackground` property is the canonical approach (used by wry, Electron).
        let no = NSNumber::numberWithBool(false);
        let key = NSString::from_str("drawsBackground");
        let _: () = msg_send![&*webview, setValue: &*no, forKey: &*key];

        // Auto-resize to fill the panel content view.
        let _: () = msg_send![&*webview, setAutoresizingMask: 18u64]; // width|height

        let webview_ref: &objc2::runtime::AnyObject = &*webview;
        let webview_view = webview_ref as *const _ as *mut objc2::runtime::AnyObject;
        let _: () = msg_send![panel, setContentView: webview_view];

        match source {
            PageSource::Dev { url } => {
                let ns_url_str = NSString::from_str(url);
                let ns_url = NSURL::URLWithString(&ns_url_str);
                if let Some(ns_url) = ns_url {
                    let request = NSURLRequest::requestWithURL(&ns_url);
                    let _ = webview.loadRequest(&request);
                } else {
                    log::warn!("[notch-window] could not parse dev url={url}");
                }
            }
            PageSource::Bundled { index_html, root } => {
                let Ok(mut file_url) = url::Url::from_file_path(index_html) else {
                    log::warn!(
                        "[notch-window] index_html not absolute: {}",
                        index_html.display()
                    );
                    return webview;
                };
                file_url.set_query(Some("window=notch"));
                let Ok(read_access_url) = url::Url::from_file_path(root) else {
                    log::warn!(
                        "[notch-window] resource root not absolute: {}",
                        root.display()
                    );
                    return webview;
                };
                let ns_url_str = NSString::from_str(file_url.as_str());
                let read_access_str = NSString::from_str(read_access_url.as_str());
                match (
                    NSURL::URLWithString(&ns_url_str),
                    NSURL::URLWithString(&read_access_str),
                ) {
                    (Some(ns_url), Some(read_access_ns)) => {
                        let _ =
                            webview.loadFileURL_allowingReadAccessToURL(&ns_url, &read_access_ns);
                        log::info!(
                            "[notch-window] loaded bundled page index={}",
                            index_html
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("index.html")
                        );
                    }
                    _ => log::warn!(
                        "[notch-window] could not parse bundled file URLs (index={})",
                        index_html
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("index.html")
                    ),
                }
            }
        }
    }

    webview
}

// ── Core-URL injection timer ──────────────────────────────────────────────────

/// Spawn a 1 Hz repeating timer that waits for the embedded core to become
/// ready (indicated by `CoreProcessHandle` setting `OPENHUMAN_CORE_RPC_URL`
/// in the process env), then injects the base URL into the WKWebView.
///
/// After the first successful inject the timer becomes a no-op until it is
/// invalidated by `NotchPanel::order_out()` when the panel is hidden.
unsafe fn spawn_inject_timer(webview: Retained<WKWebView>) -> Retained<NSTimer> {
    let tick_count: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    let injected: Rc<Cell<bool>> = Rc::new(Cell::new(false));

    let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
        tick_count.set(tick_count.get() + 1);

        if injected.get() || tick_count.get() < PAGE_LOAD_TICKS {
            return;
        }

        let Ok(rpc_url) = std::env::var("OPENHUMAN_CORE_RPC_URL") else {
            return; // Core not ready yet — try again next tick.
        };

        // Strip `/rpc` path suffix; Socket.IO connects to the base host.
        let base_url = rpc_url.trim_end_matches("/rpc").to_string();

        // The core Socket.IO handshake rejects unauthenticated clients, and this
        // WKWebView has no Tauri IPC, so `getCoreRpcToken()` can't `invoke`. Hand
        // the per-process bearer in via a global the same way as the URL (our own
        // first-party webview — same trust as the renderer's `core_rpc_token`).
        // The token is published *after* the URL env is set (post embedded spawn),
        // so wait for it rather than injecting an empty token that gets rejected.
        let token = match crate::core_process::current_rpc_token() {
            Some(t) if !t.is_empty() => t,
            _ => return, // bearer not published yet — retry next tick
        };
        log::info!(
            "[notch-window] injecting core url + bearer (token_len={})",
            token.len()
        );

        // Set a global AND dispatch a custom event so React can pick up the URL
        // regardless of whether the component mounted before or after this fires.
        let js = format!(
            "window.__OPENHUMAN_NOTCH_CORE_TOKEN__='{token}';\
             window.__OPENHUMAN_NOTCH_CORE_URL__='{base_url}';\
             window.dispatchEvent(new CustomEvent('notch:core-url',{{detail:{{url:'{base_url}'}}}}));"
        );
        let js_str = NSString::from_str(&js);
        unsafe {
            let _: () = msg_send![
                &*webview,
                evaluateJavaScript: &*js_str,
                completionHandler: std::ptr::null::<objc2::runtime::AnyObject>()
            ];
        }

        injected.set(true);
        log::debug!("[notch-window] injected core URL base={base_url}");
    });

    unsafe {
        NSTimer::scheduledTimerWithTimeInterval_repeats_block(INJECT_POLL_SECONDS, true, &block)
    }
}
