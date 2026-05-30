// OpenHuman mobile (iOS + Android) Tauri host.
//
// No CEF runtime, no Rust core sidecar, no desktop chrome. The React app
// (built from `app/src/`) is loaded into a single WKWebView (iOS) /
// Android WebView; it talks to a remote desktop core via the TS-side
// TransportManager (LAN HTTP / encrypted tunnel / cloud HTTP — see
// `app/src/services/transport/`).

#[cfg(not(any(target_os = "ios", target_os = "android")))]
compile_error!(
    "openhuman-mobile only supports iOS and Android. Use app/src-tauri for desktop."
);

use tauri::{AppHandle, Manager, Runtime};

/// Tauri command: terminate the app cleanly. Used by the Settings page
/// "Sign out / forget device" flow when the user wants to back out of a
/// paired session.
#[tauri::command]
async fn app_quit<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    log::info!("[mobile] app_quit invoked");
    app.exit(0);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    log::info!("[mobile] run() — starting mobile Tauri builder");

    tauri::Builder::default()
        .plugin(tauri_plugin_barcode_scanner::init())
        // PTT ships Swift sources for iOS only; on Android the plugin
        // registers as a no-op stub (all commands return NotSupported).
        // See packages/tauri-plugin-ptt/src/lib.rs.
        .plugin(tauri_plugin_ptt::init())
        .invoke_handler(tauri::generate_handler![app_quit])
        .setup(|app| {
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.show();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running mobile tauri application");
}
