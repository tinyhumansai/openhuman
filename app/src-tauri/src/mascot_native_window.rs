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
//! - No Tauri IPC bridge. The mascot window uses `WKScriptMessageHandler`
//!   for the few host calls it needs (close, future: drag/clickthrough).
//!   For now we keep the page passive — toggle via the tray menu.
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
/// How often we poll the cursor position to detect hover over the mascot.
const HOVER_POLL_SECONDS: f64 = 0.05;

/// Holds the panel + webview together so we keep both alive (and drop them
/// together) for the lifetime of one show/hide cycle. The hover timer is
/// stored so we can `invalidate()` it on hide and stop firing into a
/// dropped webview.
struct MascotPanel {
    panel: Retained<NSPanel>,
    _webview: Retained<WKWebView>,
    hover_timer: Retained<NSTimer>,
}

impl MascotPanel {
    fn order_out(&self) {
        self.hover_timer.invalidate();
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

    let hover_timer = unsafe { spawn_hover_timer(panel.clone(), webview.clone()) };

    MASCOT.with(|cell| {
        *cell.borrow_mut() = Some(MascotPanel {
            panel,
            _webview: webview,
            hover_timer,
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

        // Always click-through. The panel never receives mouse events; the
        // cursor passes straight to whatever's behind it. Hover is detected
        // by polling `NSEvent::mouseLocation()` against the panel frame in
        // a Foundation timer (see `spawn_hover_timer`), and the page CSS
        // animates the mascot out of the way when the cursor is over it.
        panel.setIgnoresMouseEvents(true);

        // Don't show in the Dock / Cmd+Tab.
        let _: () = msg_send![&*panel, setExcludedFromWindowsMenu: true];
    }

    panel
}

/// Two right-edge resting spots one mascot-height apart. The mascot
/// alternates between them when the cursor catches up — small hop, not a
/// trip across the screen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Home,
    HopUp,
}

fn slot_frame(mtm: MainThreadMarker, slot: Slot) -> NSRect {
    let screen = primary_screen_frame(mtm);
    let x = screen.origin.x + screen.size.width - PANEL_SIZE - PANEL_MARGIN;
    // AppKit origin is bottom-left. `Home` sits at the bottom; `HopUp`
    // is one full panel-height above it so the mascot completely clears
    // the cursor's previous position with no visible overlap.
    let y_home = screen.origin.y + PANEL_MARGIN;
    let y = match slot {
        Slot::Home => y_home,
        Slot::HopUp => y_home + PANEL_SIZE,
    };
    NSRect::new(NSPoint::new(x, y), NSSize::new(PANEL_SIZE, PANEL_SIZE))
}

/// Schedule a repeating Foundation timer on the main run loop that polls
/// the global cursor position. When the cursor enters the mascot's panel
/// frame, the panel hops to the *other* right-edge corner with an
/// animated `setFrame:display:animate:` move so the user can keep working
/// without the mascot covering the spot they were trying to click. The
/// panel is `ignoresMouseEvents=true` regardless, so even mid-animation
/// the cursor passes straight through.
unsafe fn spawn_hover_timer(
    panel: Retained<NSPanel>,
    _webview: Retained<WKWebView>,
) -> Retained<NSTimer> {
    // Fixed reference rect: the mascot's home position. Cursor entering
    // this rect makes the panel flee to `HopUp`; leaving it brings it
    // back to `Home`. We compare against the home rect — not the panel's
    // current frame — so the cursor moving away from the original spot
    // is always what triggers the return, regardless of where the panel
    // has currently hopped to.
    let mtm_for_home = unsafe { MainThreadMarker::new_unchecked() };
    let home_rect = slot_frame(mtm_for_home, Slot::Home);
    let current_slot: Rc<Cell<Slot>> = Rc::new(Cell::new(Slot::Home));

    let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
        // Safe: this block only fires on the main run loop the timer was
        // scheduled on, which is the AppKit main thread.
        let mtm = unsafe { MainThreadMarker::new_unchecked() };

        let cursor = unsafe { NSEvent::mouseLocation() };
        let inside_home = cursor.x >= home_rect.origin.x
            && cursor.x <= home_rect.origin.x + home_rect.size.width
            && cursor.y >= home_rect.origin.y
            && cursor.y <= home_rect.origin.y + home_rect.size.height;

        let desired = if inside_home { Slot::HopUp } else { Slot::Home };
        if desired == current_slot.get() {
            return;
        }
        current_slot.set(desired);
        let target = slot_frame(mtm, desired);
        log::debug!(
            "[mascot-native] cursor {} home — moving to slot={}",
            if inside_home { "entered" } else { "left" },
            match desired {
                Slot::Home => "Home",
                Slot::HopUp => "HopUp",
            }
        );
        panel.setFrame_display_animate(target, true, true);
    });

    unsafe {
        NSTimer::scheduledTimerWithTimeInterval_repeats_block(HOVER_POLL_SECONDS, true, &block)
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
