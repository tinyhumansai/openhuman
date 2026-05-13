//! Native macOS NSPanel + WKWebView host for the floating mascot.
//!
//! The vendored tauri-cef runtime cannot render transparent windowed-mode
//! browsers (CEF clamps `BrowserSettings.background_color` alpha to 0xFF for
//! windowed browsers; only off-screen rendering supports transparency, which
//! the runtime does not enable). This module bypasses Tauri's runtime
//! entirely for the mascot: it spawns a free-floating `NSPanel`, embeds a
//! `WKWebView`, and points it at the same Vite dev URL the main window loads
//! — but with `?window=mascot` so the React entry can branch on it.
//!
//! Trade-offs:
//!
//! - macOS-only. Linux/Windows would need a parallel native webview path.
//! - No Tauri IPC bridge. Toggle via the tray menu. Drag-to-reposition is
//!   handled entirely from Rust by polling `NSEvent::pressedMouseButtons()`
//!   in the same Foundation timer that tracks the cursor.
//! - Page source is dev-server in development, bundled `file://` in
//!   production. The bundled path uses `loadFileURL:allowingReadAccessToURL:`
//!   with the resource directory as the read-access root so ESM imports
//!   from the Vite build resolve correctly.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSPanel, NSScreen, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{NSNumber, NSPoint, NSRect, NSSize, NSString, NSTimer, NSURLRequest, NSURL};
use objc2_web_kit::{WKWebView, WKWebViewConfiguration};
use tauri::{AppHandle, Manager};

use crate::AppRuntime;

/// Logical width / height of the mascot panel. The `<YellowMascot>` SVG
/// canvas is square so we keep the host square too. Down to ~79pt
/// (140 → 105 → 79) so it sits unobtrusively in the corner.
const PANEL_SIZE: f64 = 79.0;
/// Distance from the bottom-right monitor corner on first show.
const PANEL_MARGIN: f64 = 0.0;
/// How often we poll the cursor position for drag tracking (~60 fps).
const DRAG_POLL_SECONDS: f64 = 0.016;

/// Holds the panel + webview together so we keep both alive (and drop them
/// together) for the lifetime of one show/hide cycle. The drag timer is
/// stored so we can `invalidate()` it on hide and stop firing into a
/// dropped webview.
struct MascotPanel {
    panel: Retained<NSPanel>,
    webview: Retained<WKWebView>,
    drag_timer: Retained<NSTimer>,
}

impl MascotPanel {
    fn order_out(&self) {
        self.drag_timer.invalidate();
        self.panel.orderOut(None);
    }
}

thread_local! {
    /// Accessed only from the main thread (Tauri IPC commands and the tray
    /// menu callback both run on it). NSPanel/WKWebView are not Send/Sync,
    /// so a thread-local is the simplest safe storage.
    static MASCOT: RefCell<Option<MascotPanel>> = const { RefCell::new(None) };
}

/// True if a mascot panel is currently alive on this thread.
pub(crate) fn is_open() -> bool {
    MASCOT.with(|cell| cell.borrow().is_some())
}

/// Tear down the panel + webview if present.
pub(crate) fn hide() {
    MASCOT.with(|cell| {
        if let Some(existing) = cell.borrow_mut().take() {
            log::info!("[mascot-native] dropping panel");
            existing.order_out();
        }
    });
}

/// Build (or focus) the floating mascot panel.
pub(crate) fn show(app: &AppHandle<AppRuntime>) -> Result<(), String> {
    if let Some(()) = MASCOT.with(|cell| {
        cell.borrow().as_ref().map(|existing| {
            log::debug!("[mascot-native] panel already open — bringing to front");
            existing.panel.orderFrontRegardless();
        })
    }) {
        return Ok(());
    }

    let mtm = MainThreadMarker::new()
        .ok_or_else(|| "mascot show called off the main thread".to_string())?;

    let source = resolve_page_source(app)?;
    log::info!("[mascot-native] loading source={source:?}");

    let frame = bottom_right_frame(mtm);
    log::debug!(
        "[mascot-native] frame origin=({},{}) size=({},{})",
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height
    );

    let panel = unsafe { build_panel(mtm, frame) };
    let webview = unsafe { build_webview(mtm, &panel, &source) };

    panel.makeKeyAndOrderFront(None);
    panel.orderFrontRegardless();

    let drag_timer = unsafe { spawn_drag_timer(panel.clone(), webview.clone()) };

    MASCOT.with(|cell| {
        *cell.borrow_mut() = Some(MascotPanel {
            panel,
            webview,
            drag_timer,
        });
    });
    log::info!("[mascot-native] panel shown");
    Ok(())
}

/// Where the mascot's HTML lives. In dev we point WKWebView at the Vite
/// dev server; in production we point it at the bundled `index.html` on
/// disk and grant read access to its resource directory so ESM imports
/// from the Vite output resolve correctly.
#[derive(Debug)]
enum PageSource {
    Dev { url: String },
    Bundled { index_html: PathBuf, root: PathBuf },
}

fn resolve_page_source(app: &AppHandle<AppRuntime>) -> Result<PageSource, String> {
    if let Some(mut url) = app.config().build.dev_url.as_ref().cloned() {
        // Append `?window=mascot` so main.tsx can branch on URL params
        // (the panel is not part of Tauri's runtime, so
        // `getCurrentWindow().label` doesn't apply here).
        let query = url
            .query()
            .map(|q| format!("{q}&window=mascot"))
            .unwrap_or_else(|| "window=mascot".into());
        url.set_query(Some(&query));
        return Ok(PageSource::Dev {
            url: url.to_string(),
        });
    }

    // Production: walk up from `resource_dir()` looking for `index.html`.
    // The packaged layout typically puts the Vite output directly under
    // the resource dir, but tauri-bundler can nest it (e.g. under a
    // `dist/` subfolder), so we search a couple of likely spots before
    // giving up.
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
    Err(format!(
        "mascot bundled index.html not found under resource_dir={}",
        resource_dir.display()
    ))
}

/// Frame of the primary screen — the one hosting the menu bar at index
/// 0 of `NSScreen.screens`. Note that `NSScreen.mainScreen` would be
/// wrong here: it returns whichever screen has the active key window, so
/// it changes when the user moves focus between displays and would
/// reposition the panel under the cursor instead of pinning it to the
/// menu-bar host.
fn primary_screen_frame(mtm: MainThreadMarker) -> NSRect {
    let screens = NSScreen::screens(mtm);
    if let Some(primary) = screens.firstObject() {
        return primary.frame();
    }
    log::warn!("[mascot-native] NSScreen::screens returned empty — falling back to 1440x900");
    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0))
}

/// Anchor the panel to the bottom-right of the primary screen using
/// AppKit's bottom-left origin convention.
fn bottom_right_frame(mtm: MainThreadMarker) -> NSRect {
    // `frame()` is the full screen including the menu bar / Dock zones, so
    // bottom-right(0,0) lands at the absolute pixel corner — that's what
    // "extreme bottom right" wants. `visibleFrame()` would inset by Dock
    // height which leaves a gap.
    let frame = primary_screen_frame(mtm);
    let x = frame.origin.x + frame.size.width - PANEL_SIZE - PANEL_MARGIN;
    let y = frame.origin.y + PANEL_MARGIN;
    NSRect::new(NSPoint::new(x, y), NSSize::new(PANEL_SIZE, PANEL_SIZE))
}

unsafe fn build_panel(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSPanel> {
    // Borderless + NonactivatingPanel: no chrome, doesn't steal focus from
    // the user's frontmost app on click.
    let style: NSWindowStyleMask =
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let backing = NSBackingStoreType::Buffered;

    let panel: Retained<NSPanel> = unsafe {
        let allocated = NSPanel::alloc(mtm);
        msg_send![
            allocated,
            initWithContentRect: frame,
            styleMask: style,
            backing: backing,
            defer: false,
        ]
    };

    unsafe {
        // Transparency
        panel.setOpaque(false);
        let clear = NSColor::clearColor();
        panel.setBackgroundColor(Some(&clear));
        panel.setHasShadow(false);

        // Float above normal windows AND fullscreen apps. Status-bar level
        // (25) plus canJoinAllSpaces+transient is the same recipe used by
        // the existing `configure_overlay_window_macos` helper.
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

        // Click-through: the panel never receives mouse events directly.
        // Drag-to-reposition is detected by polling
        // `NSEvent::pressedMouseButtons()` + `mouseLocation()` against the
        // panel frame in a Foundation timer (see `spawn_drag_timer`).
        panel.setIgnoresMouseEvents(true);

        // Don't show in the Dock / Cmd+Tab.
        let _: () = msg_send![&*panel, setExcludedFromWindowsMenu: true];
    }

    panel
}

/// Schedule a repeating Foundation timer on the main run loop that polls
/// the global cursor position and mouse-button state. When the user holds
/// the left mouse button while the cursor is inside the panel frame, the
/// panel tracks the cursor (drag-to-reposition). On release the panel
/// stays at the new position. The panel remains `ignoresMouseEvents=true`
/// throughout — the initial click passes through to the window behind,
/// but the visual drag starts on the next timer tick (~16 ms).
unsafe fn spawn_drag_timer(
    panel: Retained<NSPanel>,
    webview: Retained<WKWebView>,
) -> Retained<NSTimer> {
    let dragging: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    // Offset from cursor to panel origin at drag start so the panel
    // doesn't snap its corner to the cursor.
    let drag_offset: Rc<Cell<(f64, f64)>> = Rc::new(Cell::new((0.0, 0.0)));
    // Track previous hover state so we only dispatch when it changes.
    let was_hovering: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    // Tick counter — suppress hover events for the first ~1s after panel
    // shows so the webview can load before we start dispatching JS.
    let tick_count: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    // ~60 ticks = ~1 second at DRAG_POLL_SECONDS (0.016s).
    const HOVER_SUPPRESS_TICKS: u32 = 60;

    let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
        let cursor = NSEvent::mouseLocation();
        let left_down: bool = {
            let buttons: u64 = unsafe { msg_send![objc2::class!(NSEvent), pressedMouseButtons] };
            buttons & 1 != 0
        };
        let frame = panel.frame();
        let cursor_in_panel = cursor.x >= frame.origin.x
            && cursor.x <= frame.origin.x + frame.size.width
            && cursor.y >= frame.origin.y
            && cursor.y <= frame.origin.y + frame.size.height;

        if dragging.get() {
            if left_down {
                // Continue drag — move panel to track cursor.
                let (ox, oy) = drag_offset.get();
                let new_origin = NSPoint::new(cursor.x - ox, cursor.y - oy);
                unsafe {
                    let _: () = msg_send![&*panel, setFrameOrigin: new_origin];
                }
            } else {
                // Mouse released — end drag, panel stays put.
                dragging.set(false);
                let pos = panel.frame().origin;
                log::debug!("[mascot-native] drag ended at ({:.0},{:.0})", pos.x, pos.y);
            }
        } else if cursor_in_panel && left_down {
            // Start drag.
            dragging.set(true);
            drag_offset.set((cursor.x - frame.origin.x, cursor.y - frame.origin.y));
            log::debug!("[mascot-native] drag started");
        }

        // Hover detection: use a circular hitbox (radius = half panel size)
        // instead of the full AABB so corners don't trigger false positives.
        // Suppress for the first ~1s so the webview can finish loading.
        tick_count.set(tick_count.get().saturating_add(1));
        let center_x = frame.origin.x + frame.size.width / 2.0;
        let center_y = frame.origin.y + frame.size.height / 2.0;
        let dx = cursor.x - center_x;
        let dy = cursor.y - center_y;
        let radius = frame.size.width / 2.0;
        let cursor_in_circle = (dx * dx + dy * dy) <= (radius * radius);
        let hovering_now = cursor_in_circle
            && !left_down
            && !dragging.get()
            && tick_count.get() > HOVER_SUPPRESS_TICKS;
        if hovering_now != was_hovering.get() {
            was_hovering.set(hovering_now);
            let js_str = if hovering_now {
                "window.dispatchEvent(new CustomEvent('mascot:hover-state',{detail:{hovering:true}}))"
            } else {
                "window.dispatchEvent(new CustomEvent('mascot:hover-state',{detail:{hovering:false}}))"
            };
            log::debug!("[mascot-native] hover-state hovering={hovering_now}");
            let js = NSString::from_str(js_str);
            unsafe {
                let _: () = msg_send![
                    &*webview,
                    evaluateJavaScript: &*js,
                    completionHandler: std::ptr::null::<objc2::runtime::AnyObject>()
                ];
            }
        }
    });

    unsafe {
        NSTimer::scheduledTimerWithTimeInterval_repeats_block(DRAG_POLL_SECONDS, true, &block)
    }
}

unsafe fn build_webview(
    mtm: MainThreadMarker,
    panel: &NSPanel,
    source: &PageSource,
) -> Retained<WKWebView> {
    let config: Retained<WKWebViewConfiguration> = unsafe {
        let alloc = WKWebViewConfiguration::alloc(mtm);
        msg_send![alloc, init]
    };

    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(PANEL_SIZE, PANEL_SIZE));
    let webview: Retained<WKWebView> =
        unsafe { WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config) };

    unsafe {
        // Critical: turn off WKWebView's own background painting. Without
        // this, the webview paints the system background color underneath
        // the page even when both the panel and the page CSS are
        // transparent. There is no public Swift/ObjC API for this on
        // macOS — KVC against the private `drawsBackground` property is
        // the canonical workaround (used by wry, wkwebview-rs, Electron).
        let no = NSNumber::numberWithBool(false);
        let key = NSString::from_str("drawsBackground");
        let _: () = msg_send![&*webview, setValue: &*no, forKey: &*key];

        // Auto-resize to fill the panel content view.
        let _: () = msg_send![&*webview, setAutoresizingMask: 18u64]; // width|height

        // Make the webview the panel's content view so it fills the frame.
        let webview_ref: &objc2::runtime::AnyObject = &*webview;
        let webview_view: *mut objc2::runtime::AnyObject =
            webview_ref as *const _ as *mut objc2::runtime::AnyObject;
        let _: () = msg_send![panel, setContentView: webview_view];

        // Kick off the load.
        match source {
            PageSource::Dev { url } => {
                let ns_url_str = NSString::from_str(url);
                let ns_url: Option<Retained<NSURL>> = NSURL::URLWithString(&ns_url_str);
                if let Some(ns_url) = ns_url {
                    let request = NSURLRequest::requestWithURL(&ns_url);
                    let _ = webview.loadRequest(&request);
                } else {
                    log::warn!("[mascot-native] could not parse dev url={url}");
                }
            }
            PageSource::Bundled { index_html, root } => {
                // `loadFileURL:allowingReadAccessToURL:` is the only path
                // that lets a WKWebView resolve ESM imports from a local
                // build — `loadRequest` with a `file://` URL forbids
                // cross-origin sub-resource loads, which Vite's chunk
                // graph triggers immediately.
                let Ok(mut file_url) = url::Url::from_file_path(index_html) else {
                    log::warn!(
                        "[mascot-native] index_html is not absolute: {}",
                        index_html.display()
                    );
                    return webview;
                };
                // Same `?window=mascot` branching trick as the dev path —
                // `window.location.search` will see it on the file URL.
                file_url.set_query(Some("window=mascot"));
                let Ok(read_access_url) = url::Url::from_file_path(root) else {
                    log::warn!(
                        "[mascot-native] resource root is not absolute: {}",
                        root.display()
                    );
                    return webview;
                };
                let ns_url_str = NSString::from_str(file_url.as_str());
                let read_access_str = NSString::from_str(read_access_url.as_str());
                let ns_url = NSURL::URLWithString(&ns_url_str);
                let read_access_ns = NSURL::URLWithString(&read_access_str);
                match (ns_url, read_access_ns) {
                    (Some(ns_url), Some(read_access_ns)) => {
                        let _ =
                            webview.loadFileURL_allowingReadAccessToURL(&ns_url, &read_access_ns);
                        log::info!(
                            "[mascot-native] loaded bundled index={} root={}",
                            index_html.display(),
                            root.display()
                        );
                    }
                    _ => log::warn!(
                        "[mascot-native] could not parse bundled file URLs index={} root={}",
                        file_url,
                        read_access_url
                    ),
                }
            }
        }
    }

    webview
}
