//! Cross-module events for webview notifications.
//!
//! v1 is deliberately empty: the Tauri shell owns the CEF IPC hook and
//! fires notifications directly to the frontend over the Tauri event
//! bus (`webview-notification:fired`). When follow-up phases need core
//! subscribers (e.g. archiving notification history into the memory
//! store) they land here as `EventHandler` implementations wired from
//! the singleton bus.
