//! Keep the main app webview on the app.
//!
//! The main window has no browser chrome (no back button, no address bar), so
//! a top-level load of a remote page strands the user until the app restarts.
//! This plugin cancels any `http(s)` navigation of that webview away from the
//! app's own origin and hands the URL to the OS default browser instead.
//!
//! `app/src/utils/externalLinkGuard.ts` handles the clicks the frontend can see;
//! this is the native backstop for every route it cannot: `location.href`
//! assignments, form submits, redirects, and WebKit's `target=_blank` (which
//! asks the navigation delegate before it asks for a new window).
//!
//! Only `main` is covered. It is the one webview whose content is the app UI
//! and that can render arbitrary links; the PTT overlay renders none, and the
//! shell creates no webviews that are meant to browse remote sites.

use tauri::{plugin::TauriPlugin, Manager, Runtime};
use tauri_plugin_opener::OpenerExt;
use url::Url;

const MAIN_WINDOW: &str = "main";

/// Returns the URL to hand to the OS browser when a navigation of the webview
/// `label` must be cancelled, or `None` when the navigation may proceed.
///
/// App origins that stay in the webview:
/// - any non-`http(s)` scheme: `tauri://localhost` (macOS/Linux), `about:`,
///   `blob:`, `data:`;
/// - `<windows_scheme>://tauri.localhost` on its default port (Windows), where
///   `windows_scheme` is `https` only when the window sets `useHttpsScheme`;
///   the other scheme or any other port is a different origin (e.g. a loopback
///   service) and is handed off;
/// - `<windows_scheme>://ohwidget.localhost`, the widget sandbox origin on
///   Windows;
/// - the dev server origin, passed as `dev_url` only under `tauri dev`.
pub(crate) fn navigation_handoff(
    label: &str,
    url: &Url,
    windows_scheme: &str,
    dev_url: Option<&Url>,
) -> Option<Url> {
    if label != MAIN_WINDOW || !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    // `Url` drops a scheme's default port, so `port()` is `None` only for the
    // canonical origin.
    let is_windows_local_origin = |host: &str| {
        url.scheme() == windows_scheme && url.host_str() == Some(host) && url.port().is_none()
    };
    let is_windows_app_origin = is_windows_local_origin("tauri.localhost");
    let is_windows_widget_origin = is_windows_local_origin("ohwidget.localhost");
    if is_windows_app_origin
        || is_windows_widget_origin
        || dev_url.is_some_and(|dev| dev.origin() == url.origin())
    {
        return None;
    }
    Some(url.clone())
}

pub(crate) fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("external-navigation")
        .on_navigation(|webview, url| {
            let dev_url = if tauri::is_dev() {
                webview.config().build.dev_url.clone()
            } else {
                None
            };
            let https = webview
                .config()
                .app
                .windows
                .iter()
                .any(|w| w.label == webview.label() && w.use_https_scheme);
            let windows_scheme = if https { "https" } else { "http" };
            let Some(target) =
                navigation_handoff(webview.label(), url, windows_scheme, dev_url.as_ref())
            else {
                return true;
            };
            // Origin only: the path and query can carry tokens or user content.
            let origin = target.origin().ascii_serialization();
            log::info!(
                "[external-navigation] cancelled {} navigation to {origin}; opening in the OS browser",
                webview.label()
            );
            let app = webview.app_handle().clone();
            // `on_navigation` runs on the UI thread; spawning the opener keeps it responsive.
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(err) = app.opener().open_url(target.as_str(), None::<&str>) {
                    log::warn!("[external-navigation] OS opener failed for {origin}: {err}");
                }
            });
            false
        })
        .build()
}

#[cfg(test)]
#[path = "external_navigation_tests.rs"]
mod tests;
