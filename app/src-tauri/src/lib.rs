// Desktop targets: Windows, macOS, Linux. iOS + Android live in
// `app/src-tauri-mobile/`.
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
compile_error!("src-tauri host supports desktop (Windows/macOS/Linux) only. Mobile lives in app/src-tauri-mobile.");

// The shipped desktop app must always embed the real voice domain. Cargo
// features are per-crate, so `#[cfg(feature = "voice")]` here would test THIS
// crate's features, not the core's — a voice-less core is only observable via
// the core's own always-compiled facade. Without this assert the failure is
// silent and runtime-only: every `openhuman.voice_*` RPC answers "unknown
// method" and the UI blames a stale sidecar (#4901). Keep `voice` in the
// `openhuman_core` feature list in Cargo.toml to satisfy this.
const _: () = assert!(
    openhuman_core::openhuman::voice::VOICE_COMPILED_IN,
    "openhuman_core must be built with the `voice` feature: the desktop app ships voice, \
     and without it every openhuman.voice_* controller is unregistered (#4901). \
     Add \"voice\" to the openhuman_core `features` list in app/src-tauri/Cargo.toml."
);

// The shell talks to the in-process core only over http://127.0.0.1:<port>/rpc,
// so the core MUST embed the HTTP + Socket.IO transport (#5048). Same failure
// class as #4901: with `http-server` dropped the core never binds a listener
// and every RPC is unreachable — silent and runtime-only. The marker lives in
// the core's always-compiled facade (`core::http_server_status`) precisely so
// this assert can observe the core's feature state (a dependent's own
// `#[cfg(feature = ...)]` would test THIS crate's features, not the core's).
const _: () = assert!(
    openhuman_core::core::http_server_status::HTTP_SERVER_COMPILED_IN,
    "openhuman_core must be built with the `http-server` feature: the desktop app reaches \
     the core only over http://127.0.0.1:<port>/rpc, and without it the core binds no \
     listener so every RPC is unreachable (#5048). \
     Add \"http-server\" to the openhuman_core `features` list in app/src-tauri/Cargo.toml."
);

mod app_update;
// Artifact export command (#2779) — cross-platform Downloads copy. The `rfd`
// Save-As dialog that used to sit in front of it was removed with the crate.
mod artifact_commands;
mod claude_code;
mod core_process;
mod core_rpc;
#[cfg(target_os = "linux")]
mod deep_link_ipc;
#[cfg(target_os = "windows")]
mod deep_link_ipc_windows;
// Cross-platform module: the registry-reading function is windows-only, but
// the parsing helpers compile (and test) everywhere so `cargo test` on the
// developer host covers them.
mod deep_link_registration_check;
mod dictation_hotkeys;
mod directory_picker;
mod file_logging;
// Routing the frontend to a core that is not the one in this process. Leaf
// gated: with `gateways` off the commands are simply absent, which is what the
// renderer's feature detection expects — a stub that registered them and then
// failed at runtime would turn "this build cannot do that" into "that gateway
// is broken".
#[cfg(feature = "gateways")]
mod gateway;
mod imessage_scanner;
mod local_data_reset;
mod loopback_oauth;
#[cfg(target_os = "macos")]
mod mascot_native_window;
mod mcp_commands;
mod native_notifications;
#[cfg(target_os = "macos")]
mod notch_window;
mod process_kill;
mod process_recovery;
mod ptt_hotkeys;
mod ptt_overlay;
#[cfg(target_os = "windows")]
mod reset_reboot_schedule;
mod stderr_panic_hook;
mod window_state;
mod workspace_paths;

#[cfg(target_os = "macos")]
use tauri::menu::{PredefinedMenuItem, Submenu};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tauri::WindowEvent;
#[cfg(not(target_os = "linux"))]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, RunEvent, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_notification::NotificationExt;

#[cfg(any(windows, target_os = "linux"))]
use tauri_plugin_deep_link::DeepLinkExt;

#[cfg(target_os = "macos")]
use objc2::runtime::{AnyClass, AnyObject};
#[cfg(target_os = "macos")]
use objc2::ClassType;
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSPanel, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask, NSWindowTitleVisibility,
};

// Use Tauri's upstream native WebView runtime on every desktop platform.
pub(crate) type AppRuntime = tauri::Wry;

static EARLY_TEARDOWN_RAN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "macos")]
const APP_QUIT_MENU_ID: &str = "app_quit";

/// Tauri command: where the renderer should send core JSON-RPC.
///
/// This is the one call that makes every remote gateway work. Each of the
/// renderer's RPC call sites resolves its endpoint through here, so pointing
/// this at the active gateway points all of them at it — with no transport
/// abstraction threaded through the frontend.
#[tauri::command]
async fn core_rpc_url(
    desktop: tauri::State<'_, core_process::CoreProcessHandle>,
) -> Result<String, String> {
    Ok(active_rpc_endpoint(desktop.inner()).await.0)
}

/// Tauri command: return the bearer that must be sent with every core RPC
/// request as `Authorization: Bearer <token>`.
///
/// Paired with [`core_rpc_url`] and answered from the same place: a bearer
/// minted for the embedded core is meaningless to a core in a container, so
/// resolving the two independently would 401 every call to a provisioned
/// gateway.
#[tauri::command]
async fn core_rpc_token(
    state: tauri::State<'_, core_process::CoreProcessHandle>,
) -> Result<String, String> {
    Ok(active_rpc_endpoint(state.inner()).await.1)
}

/// The `(url, token)` pair, answered atomically so the two halves can never
/// describe different cores.
///
/// `core_rpc_url` and `core_rpc_token` each take their own snapshot when called
/// separately; if a gateway activation lands between two calls the renderer can
/// end up pairing A's URL with B's bearer. Fans of the two independent commands
/// therefore get this one call instead, which resolves once and returns both
/// halves from that single snapshot.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreRpcEndpoint {
    url: String,
    token: String,
}

#[tauri::command]
async fn core_rpc_endpoint(
    state: tauri::State<'_, core_process::CoreProcessHandle>,
) -> Result<CoreRpcEndpoint, String> {
    let (url, token) = active_rpc_endpoint(state.inner()).await;
    Ok(CoreRpcEndpoint { url, token })
}

/// The `(url, token)` the renderer should use, from the active gateway.
///
/// Returned as a pair from one function rather than resolved twice, so the two
/// commands above cannot answer about different cores — which is the failure
/// mode that produces a working URL with the wrong bearer.
#[cfg(feature = "gateways")]
async fn active_rpc_endpoint(desktop: &core_process::CoreProcessHandle) -> (String, String) {
    let active = gateway::registry::current(desktop).await;
    log::debug!("[auth] core rpc endpoint from gateway {}", active.id);
    (active.rpc_url, active.token.unwrap_or_default())
}

/// The embedded core's `(url, token)`.
///
/// The whole gateway surface is compiled out, so there is only ever one core to
/// answer about.
#[cfg(not(feature = "gateways"))]
async fn active_rpc_endpoint(desktop: &core_process::CoreProcessHandle) -> (String, String) {
    log::debug!("[auth] core rpc endpoint from the embedded core");
    (
        crate::core_rpc::core_rpc_url_value(),
        desktop.rpc_token().to_string(),
    )
}

#[tauri::command]
fn overlay_parent_rpc_url() -> Option<String> {
    let url = std::env::var("OPENHUMAN_CORE_RPC_URL").ok()?;
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

#[tauri::command]
fn process_diagnostics_list_owned() -> Result<Vec<process_recovery::ProcessInfo>, String> {
    match process_recovery::enumerate_openhuman_processes() {
        Ok(processes) => {
            log::info!(
                "[startup-recovery] diagnostics listed {} owned OpenHuman processes",
                processes.len()
            );
            Ok(processes)
        }
        Err(err) => {
            log::warn!("[startup-recovery] diagnostics process enumeration failed: {err}");
            Err(err)
        }
    }
}

#[allow(dead_code)] // Overlay disabled in tauri.conf.json; helper kept for future re-enable.
fn pin_overlay_bottom_right(window: &WebviewWindow<AppRuntime>) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        log::warn!("[overlay] could not resolve current monitor for positioning");
        return;
    };
    let Ok(size) = window.outer_size() else {
        log::warn!("[overlay] could not resolve overlay size for positioning");
        return;
    };

    let margin = 20i32;
    let x = monitor.position().x + monitor.size().width as i32 - size.width as i32 - margin;
    let y = monitor.position().y + monitor.size().height as i32 - size.height as i32 - margin;

    if let Err(err) = window.set_position(PhysicalPosition::new(x, y)) {
        log::warn!("[overlay] failed to pin overlay bottom-right: {err}");
    } else {
        log::info!("[overlay] pinned overlay bottom-right at {},{}", x, y);
    }
}

#[cfg(target_os = "macos")]
#[allow(dead_code)] // Overlay disabled in tauri.conf.json; helper kept for future re-enable.
fn configure_overlay_window_macos(window: &WebviewWindow<AppRuntime>) {
    // Standard NSWindow cannot float above fullscreen apps on macOS because
    // fullscreen apps run in a separate Space. Only NSPanel can do this.
    //
    // Tauri/tao hardcodes NSWindow as the window class, so we use
    // object_setClass() to reclass the existing NSWindow into an NSPanel
    // at runtime. This avoids creating a new window (which crashes because
    // Tao's window delegate is tightly coupled to the original NSWindow).
    //
    // After reclassing, we set the NonactivatingPanel style mask and
    // Transient collection behavior — matching the working Swift overlay
    // helper (accessibility/helper.rs OverlayController) which is confirmed
    // to float above fullscreen apps on macOS Sonoma.
    //
    // Previous attempts that FAILED:
    // 1. CGShieldingWindowLevel + CanJoinAllSpaces + FullScreenAuxiliary → hidden
    // 2. Window level i32::MAX-17 + Stationary → hidden
    // 3. CGS private API CGSSetWindowTags sticky bit → hidden
    // 4. object_setClass WITHOUT NonactivatingPanel style mask → hidden
    // 5. Create new NSPanel + reparent webview → CRASH (Tao delegate panic)
    //
    // See: https://github.com/tauri-apps/tauri/issues/11488

    match window.ns_window() {
        Ok(ns_window_raw) => unsafe {
            let ns_window = ns_window_raw as *mut AnyObject;

            // ── Reclass NSWindow → NSPanel ──────────────────────────
            let panel_class: *const AnyClass = NSPanel::class();
            objc2::ffi::object_setClass(ns_window, panel_class);
            log::info!("[overlay] reclassed NSWindow → NSPanel via object_setClass");

            // Cast to NSPanel for method calls
            let panel: &NSPanel = &*(ns_window as *const NSPanel);

            // ── Style mask: add NonactivatingPanel ──────────────────
            // This is the KEY piece the Swift helper uses. Without it,
            // the panel doesn't behave as a proper non-activating panel
            // and won't float above fullscreen Spaces.
            let current_style = panel.styleMask();
            panel.setStyleMask(current_style | NSWindowStyleMask::NonactivatingPanel);

            // ── Collection behavior ─────────────────────────────────
            // The Swift helper uses .canJoinAllSpaces + .transient
            // (NOT .stationary or .fullScreenAuxiliary alone).
            // Transient means the panel follows the active Space and
            // appears above fullscreen apps.
            panel.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::Transient
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );

            // ── Window level: status bar tier ───────────────────────
            // NSStatusWindowLevel = 25. The Swift helper uses .statusBar
            // which is the same value.
            panel.setLevel(25);

            // ── Panel-specific properties ───────────────────────────
            panel.setFloatingPanel(true);
            panel.setHidesOnDeactivate(false);
            panel.setBecomesKeyOnlyIfNeeded(true);
            panel.setWorksWhenModal(true);

            // Make sure it's ordered front
            panel.orderFrontRegardless();

            log::info!(
                "[overlay] NSPanel configured — level=25, \
                 NonactivatingPanel+canJoinAllSpaces+transient, \
                 floatingPanel={}, hidesOnDeactivate={}",
                panel.isFloatingPanel(),
                panel.hidesOnDeactivate(),
            );
        },
        Err(err) => {
            log::warn!("[overlay] failed to access native NSWindow handle: {err}");
        }
    }
}

/// Core update is handled by the Tauri shell auto-updater (`tauri-plugin-updater`)
/// since the core ships in-process with the app. This command is kept as a
/// no-op stub so the frontend's `checkCoreUpdate` keeps working without errors;
/// it always reports the running version as up-to-date.
#[tauri::command]
async fn check_core_update(
    _state: tauri::State<'_, core_process::CoreProcessHandle>,
) -> Result<serde_json::Value, String> {
    let version = env!("CARGO_PKG_VERSION");
    Ok(serde_json::json!({
        "running_version": version,
        "minimum_version": version,
        "outdated": false,
        "latest_version": version,
        "update_available": false,
    }))
}

/// Stub kept for frontend compatibility — use `apply_app_update` instead.
#[tauri::command]
async fn apply_core_update(
    _state: tauri::State<'_, core_process::CoreProcessHandle>,
    _app: tauri::AppHandle<AppRuntime>,
) -> Result<(), String> {
    Err("core ships in-process; use the Tauri shell updater (apply_app_update) instead".into())
}

#[tauri::command]
async fn restart_core_process(
    state: tauri::State<'_, core_process::CoreProcessHandle>,
) -> Result<(), String> {
    log::info!("[core] restart_core_process: command invoked from frontend");
    let _guard = state.inner().restart_lock().await;
    log::debug!("[core] restart_core_process: acquired restart lock");
    state.inner().restart().await
}

/// Attempt to auto-recover from a port conflict by reaping stale OpenHuman
/// processes (cross-platform) and restarting the embedded core.
///
/// Called by the BootCheckGate "Fix Automatically" button when the core is
/// unreachable due to a port conflict.
#[tauri::command]
async fn recover_port_conflict(
    state: tauri::State<'_, core_process::CoreProcessHandle>,
) -> Result<core_process::RecoveryOutcome, String> {
    log::info!("[core] recover_port_conflict: command invoked from frontend");
    let _guard = state.inner().restart_lock().await;
    log::debug!("[core] recover_port_conflict: acquired restart lock");
    let outcome = state.inner().recover_port_conflict().await;
    log::debug!(
        "[core] recover_port_conflict: result success={} message={}",
        outcome.success,
        outcome.message
    );
    Ok(outcome)
}

/// Terminate the foreign process holding the core RPC port, after the user
/// consented to the specific pid surfaced by `recover_port_conflict`.
#[tauri::command]
async fn force_quit_port_owner(
    pid: u32,
    state: tauri::State<'_, core_process::CoreProcessHandle>,
) -> Result<core_process::RecoveryOutcome, String> {
    log::info!("[core] force_quit_port_owner: command invoked for pid {pid}");
    let _guard = state.inner().restart_lock().await;
    let outcome = state.inner().force_quit_port_owner(pid).await;
    log::debug!(
        "[core] force_quit_port_owner: result success={} message={}",
        outcome.success,
        outcome.message
    );
    Ok(outcome)
}

/// Start the embedded core process on demand.
///
/// Called by the BootCheckGate (Local mode) before the version check.  The
/// core no longer auto-spawns at Tauri setup — the UI is responsible for
/// driving the lifecycle so it can surface startup failures and version
/// mismatches to the user.
///
/// Idempotent: `ensure_running` is a no-op if the core is already up.
#[tauri::command]
async fn start_core_process(
    state: tauri::State<'_, core_process::CoreProcessHandle>,
    app: tauri::AppHandle<AppRuntime>,
) -> Result<(), String> {
    log::info!("[core] start_core_process: command invoked from frontend");
    state.inner().ensure_running().await?;
    if let Some(notice) = state.inner().take_last_port_fallback_notice() {
        let body = format!(
            "OpenHuman is using port {} because {} was busy",
            notice.chosen_port, notice.preferred_port
        );
        if let Err(err) = app
            .notification()
            .builder()
            .title("OpenHuman")
            .body(&body)
            .show()
        {
            log::warn!("[core] fallback toast notification failed: {err}");
        } else {
            log::info!("[core] fallback toast shown: {body}");
        }
    }
    Ok(())
}

/// Cleanly exit the application.
///
/// Called by the BootCheckGate "Quit" button when the core is unreachable and
/// the user chooses to close the app rather than switch modes.
#[tauri::command]
async fn app_quit(app: tauri::AppHandle<AppRuntime>) -> Result<(), String> {
    log::info!("[app] app_quit: quit requested from frontend");
    app.exit(0);
    Ok(())
}

#[tauri::command]
async fn restart_app(app: tauri::AppHandle<AppRuntime>) -> Result<(), String> {
    log::info!("[app] restart_app invoked from frontend");
    // Persist main-window geometry and hide the window before exit so
    // the macOS WindowServer doesn't briefly black-out the desktop layer
    // on the (now defunct) display when the focused app dies, and so
    // the new process can land its window on the same display+position
    // the user had it on. (#900 secondary fixes)
    if let Some(window) = app.get_webview_window("main") {
        window_state::save_main(&window);
        if let Err(err) = window.hide() {
            log::warn!("[app] hide main window before restart failed: {err}");
        }
    }

    log::info!("[app] restart_app — starting early teardown before restart");
    perform_early_teardown_async(&app).await;
    log::info!("[app] restart_app — early teardown complete, restarting");

    app.restart()
}

/// Read the authoritative active user id from `active_user.toml` so the
/// frontend can seed `userScopedStorage` BEFORE redux-persist hydrates.
///
/// The previous frontend-only seed (a `localStorage` key) was bound to the
/// per-user CEF profile dir, so on every restart-driven user flip the new
/// process read whatever value the new profile's `localStorage` happened to
/// hold from a prior session — usually stale, triggering a false re-flip and
/// a restart loop. The Rust core writes `active_user.toml` atomically as part
/// of `auth_store_session`, so it's the only profile-independent source of
/// truth available to the UI at boot. Reuses
/// `config::default_root_openhuman_dir()` so the lookup honors
/// `OPENHUMAN_WORKSPACE` overrides used in test harnesses. (#900)
#[tauri::command]
fn get_active_user_id() -> Result<Option<String>, String> {
    let root = openhuman_core::openhuman::config::default_root_openhuman_dir()
        .map_err(|err| format!("resolve active-user state directory: {err}"))?;
    Ok(openhuman_core::openhuman::config::read_active_user_id(
        &root,
    ))
}

/// Information about an available shell-app update returned to the frontend.
#[derive(Debug, Clone, serde::Serialize)]
struct AppUpdateInfo {
    /// The currently-running app version (matches `tauri.conf.json::version`).
    current_version: String,
    /// True when the configured updater endpoint advertises a newer version.
    available: bool,
    /// Newer version reported by the updater endpoint, if any.
    available_version: Option<String>,
    /// Release notes / body for the new version, if the manifest provided one.
    body: Option<String>,
}

fn no_app_update_available(current_version: String) -> AppUpdateInfo {
    AppUpdateInfo {
        current_version,
        available: false,
        available_version: None,
        body: None,
    }
}

/// Probe the updater endpoint and report whether a newer shell build is available.
/// Does NOT download or install. Pair with `apply_app_update` to actually upgrade.
#[tauri::command]
async fn check_app_update(app: tauri::AppHandle<AppRuntime>) -> Result<AppUpdateInfo, String> {
    use tauri_plugin_updater::UpdaterExt;

    let current_version = app.package_info().version.to_string();
    log::info!("[app-update] check requested (current: {current_version})");

    let updater = app
        .updater()
        .map_err(|e| format!("updater plugin not initialized: {e}"))?;

    match updater.check().await {
        Ok(Some(update)) => {
            log::info!(
                "[app-update] update available: {} -> {}",
                current_version,
                update.version
            );
            Ok(AppUpdateInfo {
                current_version,
                available: true,
                available_version: Some(update.version.clone()),
                body: update.body.clone(),
            })
        }
        Ok(None) => {
            log::info!("[app-update] no update available");
            Ok(no_app_update_available(current_version))
        }
        Err(e) => {
            log::warn!(
                "[app-update] check failed; treating as no update available for this probe: {e}"
            );
            Ok(no_app_update_available(current_version))
        }
    }
}

/// Download and install the latest shell update, then relaunch.
///
/// Shuts the core sidecar down before download begins so the install step
/// (which on macOS replaces the entire `.app` bundle) does not race against
/// a live sidecar holding file handles inside `Contents/Resources/`. The
/// new bundled sidecar is launched fresh after `app.restart()`.
///
/// Emits Tauri events `app-update:status` and `app-update:progress` so the
/// frontend can show a snackbar / progress bar.
#[tauri::command]
async fn apply_app_update(
    state: tauri::State<'_, core_process::CoreProcessHandle>,
    app: tauri::AppHandle<AppRuntime>,
) -> Result<(), String> {
    use tauri::Emitter;
    use tauri_plugin_updater::UpdaterExt;

    log::info!("[app-update] manual apply_app_update invoked from frontend");

    let updater = app
        .updater()
        .map_err(|e| format!("updater plugin not initialized: {e}"))?;

    let _ = app.emit("app-update:status", "checking");

    let update = match updater.check().await {
        Ok(Some(u)) => u,
        Ok(None) => {
            log::info!("[app-update] no update available");
            let _ = app.emit("app-update:status", "up_to_date");
            return Ok(());
        }
        Err(e) => {
            log::warn!("[app-update] check failed: {e}");
            let _ = app.emit("app-update:status", "error");
            return Err(format!("update check failed: {e}"));
        }
    };

    let new_version = update.version.clone();
    log::info!(
        "[app-update] downloading {} (size hint: {:?})",
        new_version,
        update.signature
    );
    let _ = app.emit("app-update:status", "downloading");

    // Shut the core sidecar down before the install step replaces the .app.
    // We hold the restart lock until app.restart() so nothing tries to
    // respawn the sidecar from the in-flight (or freshly-replaced) bundle.
    let _guard = state.inner().restart_lock().await;
    log::debug!("[app-update] acquired core restart lock");
    state.inner().shutdown().await;

    // Bounded retry on transient download failures (Sentry TAURI-RUST-4JR). The
    // core stays shut down across attempts (the restart lock is held above); we
    // only revive it on the terminal give-up. A `Reqwest` error is always
    // download-phase — `download_and_install` verifies + installs strictly after
    // a complete download — so retrying never re-runs a partial install.
    let mut attempt: u32 = 1;
    loop {
        let progress_app = app.clone();
        let install_app = app.clone();
        let download_result = update
            .download_and_install(
                move |chunk_length, content_length| {
                    let payload = serde_json::json!({
                        "chunk": chunk_length,
                        "total": content_length,
                    });
                    let _ = progress_app.emit("app-update:progress", payload);
                },
                move || {
                    log::info!("[app-update] download complete — installing");
                    let _ = install_app.emit("app-update:status", "installing");
                },
            )
            .await;

        match download_result {
            Ok(()) => break,
            Err(e) => {
                let transient = app_update::is_transient_updater_err(&e);
                match app_update::classify(attempt, app_update::MAX_DOWNLOAD_ATTEMPTS, transient) {
                    app_update::RetryDecision::Retry => {
                        log::warn!(
                            "[app-update] download/install attempt {}/{} failed (transient): {e} — retrying",
                            attempt,
                            app_update::MAX_DOWNLOAD_ATTEMPTS
                        );
                        tokio::time::sleep(app_update::backoff_for(attempt)).await;
                        attempt += 1;
                        // Keep core down; re-assert downloading status for the next pass.
                        let _ = app.emit("app-update:status", "downloading");
                    }
                    app_update::RetryDecision::GiveUp => {
                        log::error!(
                            "[app-update] download/install failed after {} attempt(s): {e}",
                            attempt
                        );
                        // Same recovery as `install_app_update`: the .app wasn't
                        // swapped, so revive the in-process core we shut down above.
                        if let Err(start_err) = state.inner().ensure_running().await {
                            log::error!(
                                "[app-update] failed to restart core after apply error: {start_err}"
                            );
                        }
                        let _ = app.emit("app-update:status", "error");
                        return Err(format!("download_and_install failed: {e}"));
                    }
                }
            }
        }
    }

    log::info!("[app-update] install complete — relaunching");
    let _ = app.emit("app-update:status", "restarting");

    log::info!("[app-update] starting early teardown before restart");
    perform_early_teardown_async(&app).await;

    // Note: app.restart() never returns. Anything after this is unreachable.
    app.restart();
}

/// Holds an `Update` handle plus its downloaded bytes between the
/// `download_app_update` (background) and `install_app_update` (user
/// confirmed restart) commands. Sized at ~100MB on macOS for the .app
/// bundle, which is fine to keep in RAM until the user is ready.
struct PendingAppUpdate {
    update: tauri_plugin_updater::Update,
    bytes: Vec<u8>,
    version: String,
}

/// Tauri-managed state slot for the in-flight pending update. `None` means
/// "no update has been downloaded since launch"; `Some(_)` means the bytes
/// are ready and `install_app_update` can finalize without re-downloading.
#[derive(Default)]
struct PendingAppUpdateState(tokio::sync::Mutex<Option<PendingAppUpdate>>);

/// Result returned to the frontend after a download attempt.
#[derive(Debug, Clone, serde::Serialize)]
struct AppUpdateDownloadResult {
    /// True when an update was found and the bytes are now staged.
    ready: bool,
    /// Version of the staged update (if any).
    version: Option<String>,
    /// Release notes for the staged update.
    body: Option<String>,
}

/// Probe the updater endpoint and, if a newer build is advertised, download
/// the bundle bytes into memory but do NOT install. The frontend can then
/// surface a "Restart to apply" prompt at a moment that's safe for the user
/// (no in-flight conversation, etc.) before calling `install_app_update`.
///
/// Emits the same `app-update:status` and `app-update:progress` events as
/// `apply_app_update`, so the React state machine can drive a single UI off
/// either path. Status sequence: `checking` → `downloading` → `ready_to_install`,
/// or `up_to_date` / `error`.
#[tauri::command]
async fn download_app_update(
    app: tauri::AppHandle<AppRuntime>,
    state: tauri::State<'_, PendingAppUpdateState>,
) -> Result<AppUpdateDownloadResult, String> {
    use tauri::Emitter;
    use tauri_plugin_updater::UpdaterExt;

    log::info!("[app-update] download_app_update invoked from frontend");

    let updater = app
        .updater()
        .map_err(|e| format!("updater plugin not initialized: {e}"))?;

    let _ = app.emit("app-update:status", "checking");

    let update = match updater.check().await {
        Ok(Some(u)) => u,
        Ok(None) => {
            log::info!("[app-update] no update available");
            let _ = app.emit("app-update:status", "up_to_date");
            return Ok(AppUpdateDownloadResult {
                ready: false,
                version: None,
                body: None,
            });
        }
        Err(e) => {
            log::warn!("[app-update] check failed: {e}");
            let _ = app.emit("app-update:status", "error");
            return Err(format!("update check failed: {e}"));
        }
    };

    let new_version = update.version.clone();
    let body = update.body.clone();
    log::info!("[app-update] downloading {} (background)", new_version);
    let _ = app.emit("app-update:status", "downloading");

    // Bounded retry: a single transient mid-stream HTTP failure on the large
    // bundle download otherwise aborts the whole update (Sentry TAURI-RUST-4JR).
    // Retry only the network class; surface fatal/exhausted errors as before.
    let bytes = {
        let mut attempt: u32 = 1;
        loop {
            let progress_app = app.clone();
            let download_result = update
                .download(
                    move |chunk_length, content_length| {
                        let payload = serde_json::json!({
                            "chunk": chunk_length,
                            "total": content_length,
                        });
                        let _ = progress_app.emit("app-update:progress", payload);
                    },
                    || {
                        log::info!("[app-update] download complete — staging for install");
                    },
                )
                .await;

            match download_result {
                Ok(b) => break b,
                Err(e) => {
                    let transient = app_update::is_transient_updater_err(&e);
                    match app_update::classify(
                        attempt,
                        app_update::MAX_DOWNLOAD_ATTEMPTS,
                        transient,
                    ) {
                        app_update::RetryDecision::Retry => {
                            log::warn!(
                                "[app-update] download attempt {}/{} failed (transient): {e} — retrying",
                                attempt,
                                app_update::MAX_DOWNLOAD_ATTEMPTS
                            );
                            tokio::time::sleep(app_update::backoff_for(attempt)).await;
                            attempt += 1;
                            // Re-assert status so a stale "error" never lingers; the
                            // next attempt's progress callback resets the bar.
                            let _ = app.emit("app-update:status", "downloading");
                        }
                        app_update::RetryDecision::GiveUp => {
                            log::error!(
                                "[app-update] download failed after {} attempt(s): {e}",
                                attempt
                            );
                            let _ = app.emit("app-update:status", "error");
                            return Err(format!("download failed: {e}"));
                        }
                    }
                }
            }
        }
    };

    let mut slot = state.0.lock().await;
    *slot = Some(PendingAppUpdate {
        update,
        bytes,
        version: new_version.clone(),
    });
    drop(slot);

    log::info!(
        "[app-update] staged {} — awaiting user-initiated install",
        new_version
    );
    let _ = app.emit("app-update:status", "ready_to_install");

    Ok(AppUpdateDownloadResult {
        ready: true,
        version: Some(new_version),
        body,
    })
}

/// Install the previously-downloaded update bytes (staged by
/// `download_app_update`), then relaunch. Errors with `no pending update`
/// if `download_app_update` hasn't run yet — the frontend should fall back
/// to a fresh `apply_app_update` in that case.
///
/// Acquires the core restart lock + shuts the in-process core server down
/// before install, same as `apply_app_update`, so the macOS .app bundle
/// replacement does not race against a live core holding file handles.
#[tauri::command]
async fn install_app_update(
    core_state: tauri::State<'_, core_process::CoreProcessHandle>,
    pending: tauri::State<'_, PendingAppUpdateState>,
    app: tauri::AppHandle<AppRuntime>,
) -> Result<(), String> {
    use tauri::Emitter;

    log::info!("[app-update] install_app_update invoked from frontend");

    let staged = {
        let mut slot = pending.0.lock().await;
        slot.take()
    };
    let staged = match staged {
        Some(s) => s,
        None => {
            log::warn!("[app-update] install requested but no staged update");
            let _ = app.emit("app-update:status", "error");
            return Err("no pending update — call download_app_update first".into());
        }
    };

    log::info!("[app-update] installing staged version {}", staged.version);

    let _guard = core_state.inner().restart_lock().await;
    log::debug!("[app-update] acquired core restart lock");
    core_state.inner().shutdown().await;

    let _ = app.emit("app-update:status", "installing");
    if let Err(e) = staged.update.install(staged.bytes) {
        log::error!("[app-update] install failed: {e}");
        // The .app on disk wasn't replaced, so we keep running the
        // pre-install build — bring the core back up before returning
        // so the user can keep working instead of being silently offline.
        if let Err(start_err) = core_state.inner().ensure_running().await {
            log::error!("[app-update] failed to restart core after install error: {start_err}");
        }
        let _ = app.emit("app-update:status", "error");
        return Err(format!("install failed: {e}"));
    }

    log::info!("[app-update] install complete — relaunching");
    let _ = app.emit("app-update:status", "restarting");

    // Drop a post-update relaunch marker so the freshly launched process can
    // safely reap this instance if it wedges instead of exiting (issue #4395).
    #[cfg(any(target_os = "macos", target_os = "linux"))]

    log::info!("[app-update] starting early teardown before restart");
    perform_early_teardown_async(&app).await;

    // Note: app.restart() never returns. Anything after this is unreachable.
    app.restart();
}

/// Register (or re-register) the global dictation toggle hotkey.
/// Emits `dictation://toggle` to all webviews when the shortcut is pressed.
#[tauri::command]
async fn register_dictation_hotkey(
    app: AppHandle<AppRuntime>,
    shortcut: String,
) -> Result<(), String> {
    log::info!("[dictation] register_dictation_hotkey: shortcut={shortcut}");

    let old_shortcuts = {
        let state = app.state::<dictation_hotkeys::DictationHotkeyState>();
        let guard = state.0.lock().unwrap();
        guard.clone()
    };

    let expanded_shortcuts = dictation_hotkeys::expand_dictation_shortcuts(&shortcut);
    if expanded_shortcuts.is_empty() {
        return Err("Shortcut cannot be empty".to_string());
    }
    log::info!(
        "[dictation] expanded shortcuts: {}",
        expanded_shortcuts.join(", ")
    );

    // Reject overlap with the currently-registered PTT hotkey.
    let ptt_current = {
        let state = app.state::<ptt_hotkeys::PttHotkeyState>();
        let guard = state.shortcut.lock().unwrap();
        guard.clone()
    };
    if let Some(conflict) = ptt_hotkeys::first_conflict_with(&expanded_shortcuts, &ptt_current) {
        return Err(format!(
            "dictation shortcut '{conflict}' conflicts with the push-to-talk hotkey"
        ));
    }

    let register_shortcut = |shortcut_variant: &str| -> Result<(), String> {
        let app_clone = app.clone();
        app.global_shortcut()
            .on_shortcut(shortcut_variant, move |_app, _sc, event| {
                if event.state == ShortcutState::Pressed {
                    log::debug!("[dictation] hotkey pressed — emitting dictation://toggle");
                    if let Err(e) = app_clone.emit("dictation://toggle", ()) {
                        log::warn!("[dictation] emit failed: {e}");
                    }
                }
            })
            .map_err(|e| format!("Failed to register shortcut '{shortcut_variant}': {e}"))
    };

    let mut unregistered_old: Vec<String> = Vec::new();
    for old in &old_shortcuts {
        log::debug!("[dictation] unregistering previous shortcut: {old}");
        if let Err(e) = app.global_shortcut().unregister(old.as_str()) {
            for restored in &unregistered_old {
                if let Err(restore_err) = register_shortcut(restored.as_str()) {
                    log::warn!(
                        "[dictation] rollback failed while restoring old shortcut '{restored}': {restore_err}"
                    );
                }
            }
            return Err(format!(
                "Failed to unregister previous shortcut '{old}': {e}"
            ));
        }
        unregistered_old.push(old.clone());
    }

    let mut newly_registered: Vec<String> = Vec::new();
    for shortcut_variant in &expanded_shortcuts {
        if let Err(err) = register_shortcut(shortcut_variant.as_str()) {
            log::error!("[dictation] failed to register shortcut '{shortcut_variant}': {err}");
            for registered in &newly_registered {
                if let Err(unregister_err) = app.global_shortcut().unregister(registered.as_str()) {
                    log::warn!(
                        "[dictation] rollback failed while unregistering '{registered}': {unregister_err}"
                    );
                }
            }
            for old in &old_shortcuts {
                if let Err(restore_err) = register_shortcut(old.as_str()) {
                    log::warn!(
                        "[dictation] rollback failed while restoring old shortcut '{old}': {restore_err}"
                    );
                }
            }
            return Err(err);
        }
        newly_registered.push(shortcut_variant.clone());
    }

    // Persist all newly registered shortcuts.
    {
        let state = app.state::<dictation_hotkeys::DictationHotkeyState>();
        let mut guard = state.0.lock().unwrap();
        *guard = expanded_shortcuts.clone();
    }

    log::info!(
        "[dictation] shortcuts registered: {}",
        expanded_shortcuts.join(", ")
    );
    Ok(())
}

/// Unregister the global dictation hotkey (if any).
#[tauri::command]
async fn unregister_dictation_hotkey(app: AppHandle<AppRuntime>) -> Result<(), String> {
    log::info!("[dictation] unregister_dictation_hotkey: called");
    let state = app.state::<dictation_hotkeys::DictationHotkeyState>();
    let mut guard = state.0.lock().unwrap();
    if guard.is_empty() {
        log::debug!("[dictation] no shortcut registered — nothing to unregister");
    } else {
        let old_shortcuts = guard.clone();
        guard.clear();
        for old in old_shortcuts {
            log::debug!("[dictation] unregistering shortcut: {old}");
            app.global_shortcut()
                .unregister(old.as_str())
                .map_err(|e| {
                    log::warn!("[dictation] failed to unregister '{old}': {e}");
                    format!("Failed to unregister shortcut '{old}': {e}")
                })?;
            log::info!("[dictation] shortcut unregistered: {old}");
        }
    }
    Ok(())
}

/// Register (or re-register) the global push-to-talk hotkey. Emits
/// `ptt://start { session_id }` on press and `ptt://stop { session_id }`
/// on release.
#[tauri::command]
async fn register_ptt_hotkey(app: AppHandle<AppRuntime>, shortcut: String) -> Result<(), String> {
    log::info!("[ptt] register_ptt_hotkey: shortcut={shortcut}");

    let expanded = ptt_hotkeys::expand_ptt_shortcuts(&shortcut).map_err(|e| e.to_string())?;

    // Reject overlap with the currently-registered dictation hotkey.
    let dictation_current = {
        let state = app.state::<dictation_hotkeys::DictationHotkeyState>();
        let guard = state.0.lock().unwrap();
        guard.clone()
    };
    if let Some(conflict) = ptt_hotkeys::first_conflict_with(&expanded, &dictation_current) {
        return Err(ptt_hotkeys::PttError::ConflictsWithDictation(conflict).to_string());
    }

    let old_shortcuts = {
        let state = app.state::<ptt_hotkeys::PttHotkeyState>();
        let guard = state.shortcut.lock().unwrap();
        guard.clone()
    };

    // Lazy-instantiate the overlay window so it's ready before the first press.
    if let Err(e) = ptt_overlay::ensure_window(&app) {
        log::warn!("[ptt] overlay window create failed (continuing): {e}");
    }

    let register_shortcut = |variant: &str| -> Result<(), String> {
        let app_pressed = app.clone();
        let app_released = app.clone();
        let variant_owned = variant.to_string();
        app.global_shortcut()
            .on_shortcut(variant, move |app_inner, _sc, event| {
                let state = app_inner.state::<ptt_hotkeys::PttHotkeyState>();
                match event.state {
                    ShortcutState::Pressed => {
                        // Drop OS key-repeat events; only the first Pressed of a hold opens a session.
                        if state
                            .is_held
                            .compare_exchange(
                                false,
                                true,
                                std::sync::atomic::Ordering::AcqRel,
                                std::sync::atomic::Ordering::Acquire,
                            )
                            .is_err()
                        {
                            log::trace!(
                                "[ptt] press dropped (already held) shortcut={variant_owned}"
                            );
                            return;
                        }
                        let session_id = state
                            .session_counter
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                            + 1;
                        log::debug!(
                            "[ptt] pressed shortcut={variant_owned} session_id={session_id}"
                        );
                        if let Err(e) = app_pressed.emit(
                            "ptt://start",
                            serde_json::json!({
                                "session_id": session_id,
                            }),
                        ) {
                            log::warn!("[ptt] emit start failed: {e}");
                        }
                    }
                    ShortcutState::Released => {
                        if !state
                            .is_held
                            .swap(false, std::sync::atomic::Ordering::AcqRel)
                        {
                            // No corresponding Pressed in our state — stale event, drop.
                            log::trace!(
                                "[ptt] release dropped (not held) shortcut={variant_owned}"
                            );
                            return;
                        }
                        let session_id = state
                            .session_counter
                            .load(std::sync::atomic::Ordering::Relaxed);
                        log::debug!(
                            "[ptt] released shortcut={variant_owned} session_id={session_id}"
                        );
                        if let Err(e) = app_released.emit(
                            "ptt://stop",
                            serde_json::json!({
                                "session_id": session_id,
                            }),
                        ) {
                            log::warn!("[ptt] emit stop failed: {e}");
                        }
                    }
                }
            })
            .map_err(|e| format!("Failed to register ptt shortcut '{variant}': {e}"))
    };

    // Unregister previous PTT variants.
    let mut unregistered: Vec<String> = Vec::new();
    for old in &old_shortcuts {
        if let Err(e) = app.global_shortcut().unregister(old.as_str()) {
            // Rollback already-unregistered ones.
            for r in &unregistered {
                if let Err(re) = register_shortcut(r) {
                    log::warn!("[ptt] rollback failed for '{r}': {re}");
                }
            }
            return Err(format!(
                "Failed to unregister previous ptt shortcut '{old}': {e}"
            ));
        }
        unregistered.push(old.clone());
    }

    // Register the new variants. Rollback on first failure.
    let mut newly_registered: Vec<String> = Vec::new();
    for v in &expanded {
        if let Err(e) = register_shortcut(v) {
            for r in &newly_registered {
                if let Err(re) = app.global_shortcut().unregister(r.as_str()) {
                    log::warn!("[ptt] rollback failed for '{r}': {re}");
                }
            }
            for old in &old_shortcuts {
                if let Err(re) = register_shortcut(old) {
                    log::warn!("[ptt] rollback failed for '{old}': {re}");
                }
            }
            return Err(e);
        }
        newly_registered.push(v.clone());
    }

    {
        let state = app.state::<ptt_hotkeys::PttHotkeyState>();
        let mut guard = state.shortcut.lock().unwrap();
        *guard = expanded.clone();
    }

    log::info!("[ptt] registered: {}", expanded.join(", "));
    Ok(())
}

/// Unregister the global PTT hotkey (if any).
#[tauri::command]
async fn unregister_ptt_hotkey(app: AppHandle<AppRuntime>) -> Result<(), String> {
    log::info!("[ptt] unregister_ptt_hotkey: called");
    let state = app.state::<ptt_hotkeys::PttHotkeyState>();
    let old = {
        let guard = state.shortcut.lock().unwrap();
        guard.clone()
    };
    let mut still_registered: Vec<String> = Vec::new();
    for s in &old {
        if let Err(e) = app.global_shortcut().unregister(s.as_str()) {
            log::warn!("[ptt] unregister '{s}' failed: {e}");
            still_registered.push(s.clone());
        }
    }
    // Only retain variants that genuinely failed to unregister; the rest are gone.
    {
        let mut guard = state.shortcut.lock().unwrap();
        *guard = still_registered;
    }
    // Destroy the overlay window so resources are released.
    ptt_overlay::destroy_window(&app);
    Ok(())
}

fn is_daemon_mode() -> bool {
    std::env::args().any(|arg| arg == "daemon" || arg == "--daemon")
}

/// Returns true when an executable named `name` is discoverable on `$PATH`.
///
/// Inline `which`-style lookup so the deep-link pre-flight on Linux can
/// skip `tauri-plugin-deep-link::register_all` cleanly when `xdg-mime` is
/// missing (OPENHUMAN-TAURI-AS). Walks `$PATH` entries, joins `name`, and
/// returns true on the first hit that is a regular file. The metadata check
/// is `is_file()` rather than an executable-bit check: on Linux any file in
/// `$PATH` that is named like the binary is enough to gate the plugin call
/// (the plugin itself will surface the real exec error to its own warn),
/// and an executable-bit check would require unix-specific
/// `MetadataExt::mode` plumbing that isn't worth the platform branch for a
/// single discoverability gate.
#[cfg(target_os = "linux")]
fn path_has_executable(name: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|dir| dir.join(name).is_file())
}

/// Tauri command: swap the main window's title bar to match the sidebar state.
///
/// Expanded, the sidebar's own header stands in for a title bar: it is the
/// drag region, and its icon row is aligned with the traffic lights (see
/// `trafficLightPosition` in `tauri.conf.json` and the arithmetic in
/// `SidebarHeader.tsx`). Collapsed, that row is gone — the rail is 56px of
/// icons — so the window is left with controls floating on bare content and no
/// title anywhere. This hands the job back to macOS for the duration.
///
/// Two calls, because `TitleBarStyle` alone is not enough:
///
/// - `set_title_bar_style` toggles the *bar*. Note it does NOT move the content:
///   `tauri-runtime-wry` keeps `fullsize_content_view(true)` for both `Visible`
///   and `Overlay`, so the webview still spans the full window and nothing
///   reflows on collapse. Only `titlebar_transparent` differs.
/// - `setTitleVisibility` toggles the *text*, which the style does not touch.
///   `hiddenTitle: true` in `tauri.conf.json` is a builder-time flag (tao's
///   `with_title_hidden`) with no runtime setter anywhere in tao or tauri, so
///   the NSWindow is driven directly. Without this half the bar comes back
///   empty and the whole feature looks broken.
///
/// macOS-only: `titleBarStyle` is a no-op elsewhere, where the OS draws its own
/// decorated title bar regardless of what the sidebar is doing.
#[tauri::command]
fn set_titlebar_for_sidebar(app: AppHandle<AppRuntime>, collapsed: bool) -> Result<(), String> {
    log::debug!("[window] set_titlebar_for_sidebar collapsed={collapsed}");

    #[cfg(target_os = "macos")]
    {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "main window not found".to_string())?;

        let style = if collapsed {
            tauri::TitleBarStyle::Visible
        } else {
            tauri::TitleBarStyle::Overlay
        };
        window
            .set_title_bar_style(style)
            .map_err(|e| format!("set_title_bar_style failed: {e}"))?;

        match window.ns_window() {
            Ok(ns_window_raw) => unsafe {
                let ns_window: &NSWindow = &*(ns_window_raw as *const NSWindow);
                ns_window.setTitleVisibility(if collapsed {
                    NSWindowTitleVisibility::Visible
                } else {
                    NSWindowTitleVisibility::Hidden
                });
            },
            Err(e) => return Err(format!("ns_window unavailable: {e}")),
        }

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, collapsed);
        Ok(())
    }
}

/// Tauri command: bring the main window to front from any webview (e.g. overlay orb click).
#[tauri::command]
fn activate_main_window(app: AppHandle<AppRuntime>) -> Result<(), String> {
    log::debug!("[window] activate_main_window called from overlay");
    show_main_window(&app)
}

/// Show the floating mascot. macOS: native NSPanel + WKWebView (so the
/// window is actually transparent — vendored tauri-cef can't render
/// transparent windowed-mode browsers). Loads the Vite dev URL in
/// development and the bundled `index.html` in production. Other OSes:
/// not yet wired up.
#[tauri::command]
fn mascot_window_show(app: AppHandle<AppRuntime>) -> Result<(), String> {
    log::info!("[mascot-window] show requested");
    #[cfg(target_os = "macos")]
    {
        mascot_native_window::show(&app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("floating mascot window is macOS-only for now".into())
    }
}

/// Hide the floating mascot.
#[tauri::command]
fn mascot_window_hide(app: AppHandle<AppRuntime>) -> Result<(), String> {
    log::info!("[mascot-window] hide requested");
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        mascot_native_window::hide();
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn mascot_native_window_is_open() -> bool {
    mascot_native_window::is_open()
}

#[cfg(all(not(target_os = "macos"), not(target_os = "linux")))]
fn mascot_native_window_is_open() -> bool {
    false
}

/// Dispatch a notch-panel mutation onto the app main thread.
///
/// The notch is a native NSPanel + WKWebView; AppKit requires it to be built
/// and torn down on the main thread (and its `thread_local` storage lives
/// there). Tauri runs these IPC command handlers on a worker thread, so we hop
/// to the main thread via `run_on_main_thread`. Fire-and-forget: notch
/// visibility is cosmetic (the frontend swallows errors), so we log the result
/// on the main thread rather than blocking the command to propagate it back.
#[cfg(target_os = "macos")]
fn dispatch_notch_on_main(
    app: AppHandle<AppRuntime>,
    op: impl FnOnce(&AppHandle<AppRuntime>) + Send + 'static,
) -> Result<(), String> {
    app.clone()
        .run_on_main_thread(move || op(&app))
        .map_err(|e| format!("run_on_main_thread dispatch failed: {e}"))
}

/// Show the notch activity indicator. macOS only — transparent NSPanel + WKWebView
/// anchored to the top-centre of the primary screen. Displays live voice and
/// agent status (listening, thinking, executing) in a pill that emerges from
/// the physical notch on supported MacBook Pros.
#[tauri::command]
fn notch_window_show(app: AppHandle<AppRuntime>) -> Result<(), String> {
    log::info!("[notch-window] show requested");
    #[cfg(target_os = "macos")]
    {
        dispatch_notch_on_main(app, |app| {
            if let Err(e) = notch_window::show(app) {
                log::warn!("[notch-window] show failed: {e}");
            }
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(()) // No-op on non-macOS
    }
}

/// Hide the notch activity indicator.
#[tauri::command]
fn notch_window_hide(app: AppHandle<AppRuntime>) -> Result<(), String> {
    log::info!("[notch-window] hide requested");
    #[cfg(target_os = "macos")]
    {
        dispatch_notch_on_main(app, |_app| notch_window::hide())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(())
    }
}

/// Hide or show the OS top-level main-window frame on Windows by enumerating
/// this process's top-level windows and matching the visible
/// `Chrome_WidgetWin_1` host. `WebviewWindow::hwnd()` from the vendored CEF
/// runtime returns a `cef::Window` internal handle that `ShowWindow` rejects,
/// so we walk the OS window list instead (#1607). Empirically there is one
/// matching top-level frame; the single-instance lock window uses class
/// `com.openhuman.app-sic` and is excluded.
///
/// `SW_HIDE` removes the frame from screen AND taskbar — full hide-to-tray as
/// PR #1548 intended. On restore, the IsWindowVisible filter excludes hidden
/// windows, so we also accept currently-hidden Chrome_WidgetWin_1 frames when
/// `hide=false`.
#[cfg(target_os = "windows")]
fn set_main_window_hidden(hide: bool) {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, ShowWindow,
    };

    struct EnumCtx {
        target_pid: u32,
        hide: bool,
        require_visible: bool,
        matched: u32,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = unsafe { &mut *(lparam as *mut EnumCtx) };
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid != ctx.target_pid {
            return 1;
        }
        if ctx.require_visible && unsafe { IsWindowVisible(hwnd) } == 0 {
            return 1;
        }
        let mut class_buf = [0u16; 64];
        let n = unsafe { GetClassNameW(hwnd, class_buf.as_mut_ptr(), class_buf.len() as i32) };
        if n <= 0 {
            return 1;
        }
        let class = OsString::from_wide(&class_buf[..n as usize])
            .to_string_lossy()
            .into_owned();
        if class != "Chrome_WidgetWin_1" {
            return 1;
        }
        // Choose the restore command per frame: a minimized frame needs
        // SW_RESTORE to un-minimize, but a hidden-but-maximized frame must be
        // shown with SW_SHOW so it stays maximized (#4818). IsIconic is only
        // consulted on the restore path.
        let is_iconic = !ctx.hide && unsafe { IsIconic(hwnd) } != 0;
        let action = window_show_command(ctx.hide, is_iconic);
        unsafe { ShowWindow(hwnd, action) };
        ctx.matched += 1;
        1
    }

    let mut ctx = EnumCtx {
        target_pid: std::process::id(),
        hide,
        // Hide path: only touch currently-visible frames. Show path: also
        // pick up frames already in the SW_HIDE state.
        require_visible: hide,
        matched: 0,
    };
    unsafe { EnumWindows(Some(enum_proc), &mut ctx as *mut _ as LPARAM) };
    log::info!(
        "[window-hide] EnumWindows: action={} matched={} pid={}",
        if hide {
            "SW_HIDE"
        } else {
            "restore(SW_RESTORE/SW_SHOW)"
        },
        ctx.matched,
        ctx.target_pid,
    );
}

/// `ShowWindow` command for [`set_main_window_hidden`], chosen per frame.
///
/// Restore is *not* a single command — it depends on whether the frame is
/// iconic (minimized):
///
/// - **Minimized frame** (`is_iconic`): `SW_RESTORE`. `SW_SHOW` displays a
///   window in its *current* state, so a minimized `Chrome_WidgetWin_1` frame
///   stays minimized — clicking the desktop shortcut / taskbar entry while the
///   app was minimized did nothing, because the second-instance callback ran a
///   no-op `SW_SHOW` and then `WebviewWindow::unminimize()`, which the vendored
///   CEF runtime routes to the internal `cef::Window` proxy handle rather than
///   the visible top-level frame (#1607), so neither un-minimized the OS window
///   (#4809). `SW_RESTORE` un-minimizes AND un-hides.
/// - **Hidden-but-not-minimized frame** (the plain hide-to-tray case, which may
///   be **maximized**): `SW_SHOW`. `SW_RESTORE` would force a maximized frame
///   back to its normal size, so a window closed to the tray while maximized
///   would come back un-maximized. `SW_SHOW` displays it in its current state,
///   preserving the maximized geometry (chatgpt-codex review on #4809/#4818).
///
/// Split out as a pure `i32`-returning helper so the command selection is unit
/// testable without driving real windows.
#[cfg(target_os = "windows")]
const fn window_show_command(hide: bool, is_iconic: bool) -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_RESTORE, SW_SHOW};
    if hide {
        SW_HIDE
    } else if is_iconic {
        SW_RESTORE
    } else {
        SW_SHOW
    }
}

/// Look up the main `WebviewWindow`, optionally waiting briefly on Windows
/// for the Tauri runtime to re-track the window after SW_SHOW.
///
/// Why this exists (OPENHUMAN-TAURI-3A): on Windows the close button routes
/// through [`set_main_window_hidden`] which uses raw-HWND `SW_HIDE`. CEF
/// treats the hidden host as gone and the Tauri runtime drops its
/// `WebviewWindow` record for `"main"` until the next event-loop tick after
/// SW_SHOW restores visibility. A tray "Show window" callback that runs
/// `set_main_window_hidden(false)` and then immediately calls
/// `app.get_webview_window("main")` can race the re-track step and observe
/// `None` even though the OS window is visible — Sentry sees a
/// `[tray] failed to show main window from menu: main window not found`
/// warn even though, from the user's perspective, the window came back.
///
/// Bounded retry budget: up to 5 lookups with 10 ms between attempts (≤ 50 ms
/// worst case). The tray menu is closed during this window, so the small
/// blocking delay is invisible. After the budget expires the original
/// error path still triggers, preserving the signal if the runtime never
/// re-tracks (which would indicate a real lifecycle bug, not a race).
///
/// Non-Windows platforms use a single lookup — the close-to-tray flow that
/// produces the race is Windows-specific (the macOS close button routes
/// through `app.hide()` per PR #2049, and Linux/X11 keeps the
/// `WebviewWindow` record across `WM_DELETE_WINDOW` handling).
fn get_main_webview_window_with_retry(
    app: &AppHandle<AppRuntime>,
) -> Option<tauri::WebviewWindow<AppRuntime>> {
    #[cfg(target_os = "windows")]
    {
        const ATTEMPTS: usize = 5;
        const BACKOFF: std::time::Duration = std::time::Duration::from_millis(10);
        for attempt in 0..ATTEMPTS {
            if let Some(window) = app.get_webview_window("main") {
                if attempt > 0 {
                    log::debug!(
                        "[show_main_window] runtime re-tracked main window after {} retries",
                        attempt
                    );
                }
                return Some(window);
            }
            if attempt + 1 < ATTEMPTS {
                std::thread::sleep(BACKOFF);
            }
        }
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        app.get_webview_window("main")
    }
}

fn show_main_window(app: &AppHandle<AppRuntime>) -> Result<(), String> {
    // On Windows: surface the OS top-level Chrome_WidgetWin_1 frame BEFORE
    // any Tauri lookups. After our close handler's SW_HIDE the runtime
    // briefly drops the `WebviewWindow` record for "main" (CEF treats the
    // hidden host as gone), so `get_webview_window("main")` returns None
    // and the early `?` below would abort before SW_SHOW fires (#1607).
    // EnumWindows + SW_SHOW operates directly on the OS HWND that
    // survived independently, and the runtime re-tracks the window once
    // it's visible again — but re-tracking lands on the next event-loop
    // tick, not synchronously with SW_SHOW. `get_main_webview_window_with_retry`
    // bounds the wait to ~50 ms total so the tray callback can pick up the
    // re-tracked window without re-emitting OPENHUMAN-TAURI-3A.
    #[cfg(target_os = "windows")]
    {
        set_main_window_hidden(false);
        if let Some(webview) = app.get_webview("main") {
            let _ = webview.show();
            let _ = webview.set_focus();
        }
    }
    let window = get_main_webview_window_with_retry(app)
        .ok_or_else(|| "main window not found".to_string())?;
    window
        .show()
        .map_err(|err| format!("failed to show main window: {err}"))?;
    window
        .unminimize()
        .map_err(|err| format!("failed to unminimize main window: {err}"))?;
    window
        .set_focus()
        .map_err(|err| format!("failed to focus main window: {err}"))?;
    // `WebviewWindow::set_focus` only dispatches `WindowMessage::SetFocus`
    // (vendor/tauri-cef cef_impl.rs `WindowMessage::SetFocus` → `window.request_focus()`),
    // which lifts the OS window but does NOT call `CefBrowserHost::SetFocus(true)`.
    // Without that CEF-level focus call the renderer never gets wired as the
    // keyboard input target on cold launch: the chat textarea accepts focus
    // (cursor blinks) but `WM_KEYDOWN` messages aren't forwarded to it, so
    // typing is silently dead until the user click-outside / click-back
    // triggers `WM_KILLFOCUS`+`WM_SETFOCUS` and CEF's window handler routes
    // through `host.set_focus(1)` internally.
    //
    // Explicitly dispatch `WebviewMessage::SetFocus` (cef_impl.rs handler
    // for that variant), which is what actually invokes
    // `CefBrowserHost::SetFocus(true)`.
    let webview: &tauri::Webview<AppRuntime> = window.as_ref();
    if let Err(err) = webview.set_focus() {
        log::warn!(
            "[show_main_window] CEF webview set_focus failed (non-fatal — \
             keyboard routing may not initialize until user click-outside-and-back): {err}"
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_app_menu(app: &AppHandle<AppRuntime>) -> tauri::Result<Menu<AppRuntime>> {
    let about = PredefinedMenuItem::about(app, None, None)?;
    let hide = PredefinedMenuItem::hide(app, None)?;
    let hide_others = PredefinedMenuItem::hide_others(app, None)?;
    let show_all = PredefinedMenuItem::show_all(app, None)?;
    let quit = MenuItem::with_id(
        app,
        APP_QUIT_MENU_ID,
        "Quit OpenHuman",
        true,
        Some("CmdOrCtrl+Q"),
    )?;
    let app_sep_1 = PredefinedMenuItem::separator(app)?;
    let app_sep_2 = PredefinedMenuItem::separator(app)?;
    let app_menu = Submenu::with_items(
        app,
        "OpenHuman",
        true,
        &[
            &about,
            &app_sep_1,
            &hide,
            &hide_others,
            &show_all,
            &app_sep_2,
            &quit,
        ],
    )?;

    let undo = PredefinedMenuItem::undo(app, None)?;
    let redo = PredefinedMenuItem::redo(app, None)?;
    let cut = PredefinedMenuItem::cut(app, None)?;
    let copy = PredefinedMenuItem::copy(app, None)?;
    let paste = PredefinedMenuItem::paste(app, None)?;
    let select_all = PredefinedMenuItem::select_all(app, None)?;
    let edit_sep_1 = PredefinedMenuItem::separator(app)?;
    let edit_sep_2 = PredefinedMenuItem::separator(app)?;
    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &undo,
            &redo,
            &edit_sep_1,
            &cut,
            &copy,
            &paste,
            &edit_sep_2,
            &select_all,
        ],
    )?;

    let close = PredefinedMenuItem::close_window(app, None)?;
    let minimize = PredefinedMenuItem::minimize(app, None)?;
    let fullscreen = PredefinedMenuItem::fullscreen(app, None)?;
    let window_menu = Submenu::with_items(app, "Window", true, &[&close, &minimize, &fullscreen])?;

    Menu::with_items(app, &[&app_menu, &edit_menu, &window_menu])
}

#[cfg(target_os = "linux")]
fn setup_tray(app: &AppHandle<AppRuntime>) -> tauri::Result<()> {
    let _ = app;
    log::warn!(
        "[tray] skipping tray setup on linux: tray menu creation still panics inside GTK during packaged runs"
    );
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn setup_tray(app: &AppHandle<AppRuntime>) -> tauri::Result<()> {
    log::info!("[tray] setting up tray icon");

    let show_item = MenuItem::with_id(
        app,
        "tray_show_window",
        "Open OpenHuman",
        true,
        None::<&str>,
    )?;
    let quit_item = MenuItem::with_id(app, "tray_quit", "Quit", true, None::<&str>)?;
    // The floating mascot has a native NSPanel + WKWebView host, so the
    // tray entry only does anything on macOS. Don't surface a menu item
    // on Windows that's guaranteed to error — gate it to the platform
    // where `mascot_window_show` actually works.
    #[cfg(target_os = "macos")]
    let menu = {
        let mascot_item = MenuItem::with_id(
            app,
            "tray_toggle_mascot",
            "Toggle floating mascot",
            true,
            None::<&str>,
        )?;
        Menu::with_items(app, &[&show_item, &mascot_item, &quit_item])?
    };
    #[cfg(not(target_os = "macos"))]
    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".to_string()))?;

    TrayIconBuilder::with_id("openhuman-tray")
        .icon(icon)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray_show_window" => {
                log::info!("[tray] action=show_window source=menu");
                if let Err(err) = show_main_window(app) {
                    log::warn!("[tray] failed to show main window from menu: {err}");
                }
            }
            "tray_toggle_mascot" => {
                log::info!("[tray] action=toggle_mascot source=menu");
                if mascot_native_window_is_open() {
                    if let Err(err) = mascot_window_hide(app.clone()) {
                        log::error!("[tray] failed to hide mascot window: {err}");
                    }
                } else if let Err(err) = mascot_window_show(app.clone()) {
                    log::error!("[tray] failed to show mascot window: {err}");
                }
            }
            "tray_quit" => {
                log::info!("[tray] action=quit source=menu");
                shutdown_app_sync(app, 0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                log::info!("[tray] action=show_window source=left_click");
                if let Err(err) = show_main_window(tray.app_handle()) {
                    log::warn!("[tray] failed to show main window from tray click: {err}");
                }
            }
        })
        .build(app)?;

    log::info!("[tray] tray icon ready");
    Ok(())
}

fn shutdown_imessage_scanner<R: tauri::Runtime>(app: &AppHandle<R>) {
    if let Some(registry) = app.try_state::<std::sync::Arc<imessage_scanner::ScannerRegistry>>() {
        registry.inner().shutdown();
    }
}

/// Shared early teardown logic run before the runtime shuts down, to prevent
/// races and zombie processes.
///
/// Synchronous entry used from `RunEvent::ExitRequested` and tray quit. We intentionally
/// **do not** poll here with `std::thread::sleep` — that would block the Tauri main
/// event loop and prevent close messages from being processed. Use
/// [`perform_early_teardown_async`] when an async caller can await without
/// starving the UI loop.
fn perform_early_teardown_sync(app_handle: &AppHandle<AppRuntime>) {
    log::info!("[app] perform_early_teardown_sync — early teardown");

    shutdown_imessage_scanner(app_handle);

    // A provisioned gateway is a container or a remote process this app
    // started. Nothing else will stop it: an SSH tunnel dies with this process
    // but the box on the far side does not, and a container left running holds
    // a port and a workspace until someone finds it by hand. Blocking is right
    // here for the same reason the core's terminate signal is — this is the
    // last moment the handles still exist.
    #[cfg(feature = "gateways")]
    tauri::async_runtime::block_on(gateway::registry::shutdown());

    if let Some(core) = app_handle.try_state::<core_process::CoreProcessHandle>() {
        let core = core.inner().clone();
        // Aborts the embedded server task. Synchronous and safe on
        // the UI thread — `JoinHandle::abort` returns immediately.
        tauri::async_runtime::block_on(async move {
            core.send_terminate_signal().await;
        });
    }

    log::info!("[app] perform_early_teardown_sync — early teardown complete");
}

fn perform_early_teardown_sync_once(app_handle: &AppHandle<AppRuntime>, reason: &str) {
    if EARLY_TEARDOWN_RAN.swap(true, std::sync::atomic::Ordering::SeqCst) {
        log::info!(
            "[app] perform_early_teardown_sync_once — already ran, skipping reason={reason}"
        );
        return;
    }

    log::info!("[app] perform_early_teardown_sync_once — reason={reason}");
    perform_early_teardown_sync(app_handle);
}

/// Shared early teardown logic run before the runtime shuts down, to prevent
/// races and zombie processes.
/// Asynchronous version to be called from async Tauri commands (e.g. `restart_app`, updates).
async fn perform_early_teardown_async(app_handle: &AppHandle<AppRuntime>) {
    log::info!("[app] perform_early_teardown_async — early teardown");

    shutdown_imessage_scanner(app_handle);

    // Same teardown as the synchronous path, awaited rather than blocked on:
    // `block_on` inside an async fn runs a runtime inside a runtime.
    #[cfg(feature = "gateways")]
    gateway::registry::shutdown().await;

    if let Some(core) = app_handle.try_state::<core_process::CoreProcessHandle>() {
        let core = core.inner().clone();
        core.send_terminate_signal().await;
    }

    log::info!("[app] perform_early_teardown_async — early teardown complete");
}

/// Explicitly wind down CEF and the core before exiting from desktop-owned
/// quit actions. Linux does not build the tray or macOS application menu.
#[cfg(not(target_os = "linux"))]
fn shutdown_app_sync(app_handle: &AppHandle<AppRuntime>, exit_code: i32) {
    log::info!("[app] shutdown_app_sync — starting early teardown");
    perform_early_teardown_sync_once(app_handle, "shutdown_app_sync");
    log::info!("[app] shutdown_app_sync — early teardown complete, exiting");
    app_handle.exit(exit_code);
}

#[cfg(target_os = "linux")]
const WSL_X11_DESKTOP_WARNING: &str = "[startup] likely unsupported desktop environment: WSL with classic X11 forwarding detected (DISPLAY is set, but WAYLAND_DISPLAY/WSLg markers are absent). OpenHuman's Tauri/CEF desktop flow is fragile in this setup; use native Windows development or Windows 11 WSLg for desktop GUI work.";

#[cfg(any(target_os = "linux", test))]
fn should_warn_for_wsl_x11_desktop(
    is_wsl: bool,
    display_set: bool,
    wayland_display_set: bool,
    wslg_marker_set: bool,
) -> bool {
    is_wsl && display_set && !wayland_display_set && !wslg_marker_set
}

#[cfg(target_os = "linux")]
fn is_wsl_environment() -> bool {
    if std::env::var("WSL_DISTRO_NAME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
    {
        return true;
    }

    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|release| release.to_ascii_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn has_non_empty_env(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
}

#[cfg(target_os = "linux")]
fn has_wslg_marker() -> bool {
    has_non_empty_env("WSLG_RUNTIME_DIR") || std::path::Path::new("/mnt/wslg").exists()
}

#[cfg(target_os = "linux")]
fn warn_if_wsl_x11_desktop_launch() {
    if should_warn_for_wsl_x11_desktop(
        is_wsl_environment(),
        has_non_empty_env("DISPLAY"),
        has_non_empty_env("WAYLAND_DISPLAY"),
        has_wslg_marker(),
    ) {
        log::warn!("{WSL_X11_DESKTOP_WARNING}");
        eprintln!("{WSL_X11_DESKTOP_WARNING}");
    }
}

#[cfg(not(target_os = "linux"))]
fn warn_if_wsl_x11_desktop_launch() {}

/// Returns `true` if a display server is available on Linux.
/// Testable pure function: takes the env-presence booleans directly.
#[cfg(any(target_os = "linux", test))]
fn linux_display_server_present(display: bool, wayland_display: bool) -> bool {
    display || wayland_display
}

/// Pre-CEF display-server check for Linux (Sentry OPENHUMAN-TAURI-K1).
///
/// CEF/Chromium requires X11 (`DISPLAY`) or Wayland (`WAYLAND_DISPLAY`) to
/// initialise. Without either, `cef_initialize` returns 0 and the vendored
/// `tauri-runtime-cef` asserts `result == 1` → panic `left: 0, right: 1`.
/// This is fatal and silent on WSL2 without WSLg and on any headless Linux box.
/// Detect it here and exit with a clear message before `CefRuntime::init` runs.
#[cfg(target_os = "linux")]
fn check_linux_display_server() {
    if linux_display_server_present(
        has_non_empty_env("DISPLAY"),
        has_non_empty_env("WAYLAND_DISPLAY"),
    ) {
        log::debug!(
            "[cef-preflight] Linux display server present: DISPLAY={:?} WAYLAND_DISPLAY={:?}",
            std::env::var("DISPLAY").ok(),
            std::env::var("WAYLAND_DISPLAY").ok()
        );
        return;
    }
    let msg = "[openhuman] no display server found (DISPLAY and WAYLAND_DISPLAY are both unset).\n\
               OpenHuman requires an X11 or Wayland display to run.\n\
               On WSL2: install WSLg or configure X11 forwarding from Windows.\n\
               Set DISPLAY (e.g. export DISPLAY=:0) or WAYLAND_DISPLAY before launching.";
    log::error!(
        "[cef-preflight] Linux display server missing — CEF cannot initialize \
         (OPENHUMAN-TAURI-K1): DISPLAY and WAYLAND_DISPLAY both unset"
    );
    eprintln!("\n{msg}\n");
    std::process::exit(1);
}

#[cfg(not(target_os = "linux"))]
fn check_linux_display_server() {}

/// Pure predicate: given a candidate `DBUS_SESSION_BUS_ADDRESS` value, decide
/// whether it points to a transport `zbus` can actually open.
///
/// `tauri-plugin-single-instance` on Linux calls
/// `zbus::blocking::connection::Builder::session().unwrap()` in its `setup()`
/// closure. When `DBUS_SESSION_BUS_ADDRESS` is missing or set to `disabled`
/// (the literal string WSL2-without-WSLg sets), zbus returns
/// `Address("unsupported transport 'disabled'")` and the unwrap blows up the
/// whole process before any window is created (Sentry OPENHUMAN-TAURI-TM).
///
/// We treat an address as reachable only when at least one alternative listed
/// in the env var uses a transport `zbus` ships with: `unix:`, `tcp:`,
/// `launchd:`, or `autolaunch:`. Anything else (`disabled`, empty, unknown
/// scheme) is treated as unreachable so we can skip registering the plugin
/// instead of crashing.
#[cfg(any(target_os = "linux", test))]
fn dbus_address_is_supported(addr: &str) -> bool {
    addr.split(';').any(|entry| {
        let entry = entry.trim();
        if entry.is_empty() {
            return false;
        }
        let transport = entry.split(':').next().unwrap_or("").trim();
        matches!(transport, "unix" | "tcp" | "launchd" | "autolaunch")
    })
}

/// Pure predicate: decide whether the Linux D-Bus session bus is reachable
/// based on the relevant env vars and (optionally) the existence of the
/// `$XDG_RUNTIME_DIR/bus` socket.
///
/// Used to gate registration of `tauri-plugin-single-instance` on Linux so
/// WSL2-without-WSLg / minimal-container launches don't crash on the plugin's
/// `unwrap()` (Sentry OPENHUMAN-TAURI-TM).
#[cfg(any(target_os = "linux", test))]
fn linux_dbus_session_reachable(
    dbus_session_bus_address: Option<&str>,
    xdg_runtime_bus_socket_exists: bool,
) -> bool {
    match dbus_session_bus_address {
        Some(addr) => dbus_address_is_supported(addr),
        // When the env var is unset, zbus falls back to autolaunch which on
        // most distros requires `$XDG_RUNTIME_DIR/bus` to exist. Treat its
        // absence as "no D-Bus".
        None => xdg_runtime_bus_socket_exists,
    }
}

/// Returns `true` when this process can safely register
/// `tauri-plugin-single-instance` without tripping the
/// `Address("unsupported transport 'disabled'")` panic on Linux.
/// Always `true` on non-Linux platforms.
#[cfg(target_os = "linux")]
fn can_register_single_instance_plugin() -> bool {
    let env_addr = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok();
    let runtime_bus_present = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .map(|dir| std::path::Path::new(&dir).join("bus").exists())
        .unwrap_or(false);
    linux_dbus_session_reachable(env_addr.as_deref(), runtime_bus_present)
}

#[cfg(not(target_os = "linux"))]
fn can_register_single_instance_plugin() -> bool {
    true
}

#[cfg(any(test, feature = "e2e-test-support"))]
type CefCommandLineArg = (&'static str, Option<&'static str>);

/// Returns `true` when the process is running as root (UID 0) on Linux.
/// Testable pure function; takes the uid directly.
#[cfg(any(test, feature = "e2e-test-support"))]
fn linux_is_root_uid(uid: u32) -> bool {
    uid == 0
}

/// Whether to skip the Linux `--disable-gpu` / `--disable-gpu-compositing`
/// workaround.
///
/// The workaround was added for issue #1697 — on Arch/Manjaro-family Linux
/// systems, the AppImage can abort during CEF GPU process startup when EGL
/// context creation fails before Chromium's own fallback path gets a usable
/// renderer. The unconditional disable keeps packaged builds launchable on
/// those configurations, but the side effect is that WebGL2 surfaces (the
/// Rive mascot on the Human tab; any other GPU-accelerated canvas) cannot
/// initialise. Users on working GPU stacks (most Ubuntu, WSL2 with proper
/// driver passthrough, Fedora, etc.) can opt back into hardware
/// acceleration by setting `OPENHUMAN_FORCE_GPU=1`.
///
/// This control is explicit opt-in: `1` / `true` / `yes` / `on`
/// (case-insensitive) enables the override, and anything else (including
/// unset) preserves the default disable.
#[cfg(any(test, feature = "e2e-test-support"))]
fn cef_force_gpu_enabled(env_override: Option<&str>) -> bool {
    match env_override {
        Some(v) => {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        }
        None => false,
    }
}

/// Emergency CEF startup escape hatch for driver/runtime combinations where
/// CEF cannot initialize its GPU process before the app can render UI.
#[cfg(any(test, feature = "e2e-test-support"))]
fn cef_disable_gpu_enabled(env_override: Option<&str>) -> bool {
    match env_override {
        Some(v) => {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        }
        None => false,
    }
}

/// Pin CEF to ANGLE's pure-software SwiftShader GL backend.
///
/// SwiftShader is a software rasteriser that needs no hardware EGL/driver, so it
/// gives both WebGL surfaces and GPU-process init a working (software) context
/// on stacks where the hardware GPU path aborts. `--enable-unsafe-swiftshader`
/// is required because Chromium gates SwiftShader-backed WebGL behind it (the
/// "unsafe" label is about software perf, not security). `--disable-gpu-compositing`
/// keeps page compositing on the CPU. Crucially this does **not** pass
/// `--disable-gpu`, which would shut the GPU process down entirely — and with it
/// the SwiftShader context that lets CEF start.
#[cfg(any(test, feature = "e2e-test-support"))]
fn push_swiftshader_software_gl(args: &mut Vec<CefCommandLineArg>) {
    args.push(("--use-gl", Some("angle")));
    args.push(("--use-angle", Some("swiftshader")));
    args.push(("--enable-unsafe-swiftshader", None));
    args.push(("--disable-gpu-compositing", None));
}

#[cfg(any(test, feature = "e2e-test-support"))]
fn append_platform_cef_gpu_workarounds(
    args: &mut Vec<CefCommandLineArg>,
    os: &str,
    arch: &str,
    force_gpu_override: Option<&str>,
    disable_gpu_override: Option<&str>,
) {
    let disable_gpu = cef_disable_gpu_enabled(disable_gpu_override);

    // Issue #4294: on some Windows CEF/GPU stacks, cef::initialize can fail
    // before the app renders any UI, and user-supplied process argv switches
    // do not reach the vendored CEF runtime. Provide a narrow, release-safe
    // env escape hatch instead of forwarding arbitrary Chromium flags.
    if disable_gpu {
        if os == "windows" {
            // Issue #4385: on NVIDIA Blackwell / RTX 50-series Windows stacks the
            // bundled CEF's GPU process fails to initialise and bare
            // `--disable-gpu` is not enough — `cef::initialize` still returns 0
            // before any UI renders (#4294, #4385). `--disable-gpu` removes the
            // GPU process without giving CEF a working software GL path, so init
            // still aborts. Pin CEF to the pure-software ANGLE/SwiftShader backend
            // instead — the same fallback the Linux #1697/#4193 path relies on —
            // which needs no hardware driver and lets CEF start on GPUs the
            // bundled Chromium doesn't yet support.
            push_swiftshader_software_gl(args);
            log::info!(
                "[cef-startup] OPENHUMAN_DISABLE_GPU set on Windows: forcing ANGLE/SwiftShader software GL for CEF startup compatibility (issues #4294/#4385)"
            );
        } else {
            args.push(("--disable-gpu", None));
            args.push(("--disable-gpu-compositing", None));
            log::info!(
                "[cef-startup] OPENHUMAN_DISABLE_GPU set: adding --disable-gpu and --disable-gpu-compositing for CEF startup compatibility (issue #4294)"
            );
        }
    }

    // Issue #1697: on Arch/Manjaro-family Linux systems, the AppImage can
    // abort during CEF GPU process startup when EGL context creation fails
    // before Chromium's own fallback path gets a usable renderer.
    //
    // The original workaround disabled the GPU path with `--disable-gpu`, but
    // that shuts the GPU process down entirely — and with it every WebGL
    // surface. That regressed WebGL rendering (#4193), including the Rive
    // mascot on the Human tab.
    //
    // Instead of killing the GPU process, pin it to ANGLE's SwiftShader
    // software backend. SwiftShader is a pure-software rasteriser that needs no
    // hardware EGL/driver, so it still sidesteps the #1697 EGL-context abort,
    // while giving WebGL surfaces a working (software) context to render into.
    // `--disable-gpu-compositing` is preserved so page compositing stays on the
    // CPU exactly as before; only the lethal `--disable-gpu` is dropped.
    // `--enable-unsafe-swiftshader` is required because Chromium gates
    // SwiftShader-backed WebGL behind it (the "unsafe" label is about software
    // perf, not security). Users with working GPU stacks can still opt into
    // hardware acceleration via `OPENHUMAN_FORCE_GPU=1`.
    if os == "linux" && !disable_gpu {
        if cef_force_gpu_enabled(force_gpu_override) {
            log::info!(
                "[cef-startup] OPENHUMAN_FORCE_GPU set — skipping SwiftShader software-GL fallback (issue #1697). If the app fails to launch with a GPU process abort, unset the env var."
            );
        } else {
            push_swiftshader_software_gl(args);
            log::info!(
                "[cef-startup] Linux detected: forcing ANGLE/SwiftShader software GL so WebGL surfaces render without the crash-prone hardware GPU process (issues #1697/#4193); set OPENHUMAN_FORCE_GPU=1 for hardware acceleration"
            );
        }
    }

    // Issue #1012: Intel macOS (x86_64) crashes with EXC_CRASH (SIGABRT)
    // inside CrBrowserMain when CEF 146 tries to use GPU compositing via
    // Metal on Intel GPU hardware/drivers. Disable GPU compositing on
    // x86_64 macOS so the browser process falls back to software compositing
    // instead of aborting.
    if os == "macos" && arch == "x86_64" && !disable_gpu {
        args.push(("--disable-gpu-compositing", None));
        log::info!(
            "[cef-startup] Intel macOS detected: adding --disable-gpu-compositing (issue #1012)"
        );
    }

    // Sentry OPENHUMAN-TAURI-K1: `cef::initialize` returns 0 when running as
    // root (uid 0) on Linux unless `--no-sandbox` is passed as a command-line
    // argument. The `no_sandbox: 1` field in `cef::Settings` disables the
    // sub-process sandbox but does NOT satisfy Chromium's separate root-user
    // check in the browser process — that check requires the CLI flag.
    //
    // This hits CI / coder-bot / Docker environments (e.g.
    // `/root/.hermes/profiles/coder-bot/home`) that run as root inside a
    // container. Without the flag, `cef_initialize` returns 0 and the vendored
    // runtime assertion fires (`left: 0, right: 1`).
    #[cfg(target_os = "linux")]
    {
        let uid = nix::unistd::getuid().as_raw();
        // Dev-only: also honor OPENHUMAN_CEF_NO_SANDBOX=1 so a non-root headless
        // box (no sudo to chown chrome-sandbox root:4755) can launch over RDP.
        //
        // SECURITY: gated to debug builds only. Disabling Chromium's process
        // sandbox in a release binary would let anyone with env-write access
        // on the host silently turn off a meaningful defense-in-depth layer
        // (graycyrus review on PR #2875). In release builds `forced` stays
        // false so only the `linux_is_root_uid` path can opt in (uid=0
        // already implies root-equivalent trust).
        #[cfg(debug_assertions)]
        let forced = std::env::var("OPENHUMAN_CEF_NO_SANDBOX")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        #[cfg(not(debug_assertions))]
        let forced = false;
        if os == "linux" && (linux_is_root_uid(uid) || forced) {
            args.push(("--no-sandbox", None));
            log::info!(
                "[cef-startup] Linux: adding --no-sandbox (root uid or OPENHUMAN_CEF_NO_SANDBOX) \
                 (OPENHUMAN-TAURI-K1)"
            );
        }
    }
}

/// Whether a CEF command-line flag is `--time-ticks-at-unix-epoch` (in any
/// dash/casing form, with or without an inline `=<value>` suffix). See
/// [`strip_time_ticks_at_unix_epoch`] for why we care.
#[cfg(any(test, feature = "e2e-test-support"))]
fn is_time_ticks_at_unix_epoch_flag(flag: &str) -> bool {
    let name = flag.trim_start_matches('-');
    // Chromium switches can carry their value inline (`--flag=value`); compare
    // only the switch name so the inline form can't slip past the guard.
    let name = name.split_once('=').map_or(name, |(n, _)| n);
    name.eq_ignore_ascii_case("time-ticks-at-unix-epoch")
}

/// Issue #3554: `--time-ticks-at-unix-epoch` carries the monotonic-clock
/// origin (in microseconds) that CEF child processes — renderer / GPU /
/// utility — use to map Chromium's `TimeTicks` onto wall-clock time. Chromium
/// derives this value itself when it spawns each child, and it must stay
/// consistent with the host clock.
///
/// OpenHuman must never inject this switch into the CEF command line: a stale
/// or negative value (e.g. the `-1780937467390432` reported in #3554) pins the
/// renderer's internal clock ~56 years before the Unix epoch, which surfaces
/// as a wrong "Current Date & Time" in the app. The CEF command line is
/// assembled from several sources (the static list here plus any
/// `RuntimeInitAttribute::CommandLineArgs` contributed elsewhere), so strip the
/// flag from the final list as a guard and let Chromium compute the origin
/// locally for each process.
#[cfg(any(test, feature = "e2e-test-support"))]
fn strip_time_ticks_at_unix_epoch(args: &mut Vec<CefCommandLineArg>) {
    args.retain(|(flag, value)| {
        if is_time_ticks_at_unix_epoch_flag(flag) {
            log::warn!(
                "[cef-startup] dropping OpenHuman-supplied --time-ticks-at-unix-epoch{} so \
                 Chromium computes the clock origin locally (issue #3554)",
                value.map(|v| format!("={v}")).unwrap_or_default()
            );
            false
        } else {
            true
        }
    });
}

/// Linux only: replace Xlib's default error handler with a logging no-op.
///
/// Why: on Wayland sessions (GNOME/KDE/Hyprland) running CEF via XWayland,
/// the CEF browser process issues `XConfigureWindow` against a window the
/// XWayland server hasn't fully realized yet, and Xlib's *default* error
/// handler reacts to the resulting `BadWindow` by calling `exit(1)` — which
/// kills the entire app before the main window ever paints. This reproduces
/// reliably on Ubuntu 26.04 + GNOME-Wayland with the locally-built AppImage
/// (issue #2001 item #2 — was punted as "separate display-side concerns"
/// in PR #2032 but blocks any actual use of the app on a Wayland host).
///
/// XSetErrorHandler is a process-global registration; safe to install before
/// any X display is opened. libX11 is only pulled in transitively at *runtime*
/// (via GTK/WebKit), so the `extern` block must carry an explicit
/// `#[link(name = "X11")]` — otherwise `rust-lld` can't resolve the symbol at
/// link time and the full desktop build fails with `undefined symbol:
/// XSetErrorHandler` (only surfaces in the CI-Full / release bundle link step,
/// not CI-Lite). libX11 is present on every Linux desktop host, so linking it
/// directly is safe.
#[cfg(target_os = "linux")]
fn install_silent_x_error_handler() {
    use std::ffi::c_void;
    use std::os::raw::{c_int, c_uchar, c_ulong};
    use std::sync::Once;

    #[repr(C)]
    struct XErrorEvent {
        type_: c_int,
        display: *mut c_void,
        resourceid: c_ulong,
        serial: c_ulong,
        error_code: c_uchar,
        request_code: c_uchar,
        minor_code: c_uchar,
    }

    type ErrorHandler = unsafe extern "C" fn(*mut c_void, *mut XErrorEvent) -> c_int;
    #[link(name = "X11")]
    unsafe extern "C" {
        fn XSetErrorHandler(handler: Option<ErrorHandler>) -> Option<ErrorHandler>;
    }

    unsafe extern "C" fn silent_handler(_display: *mut c_void, ev: *mut XErrorEvent) -> c_int {
        if !ev.is_null() {
            let e = unsafe { &*ev };
            log::warn!(
                "[x11] suppressed X protocol error: code={} request={} minor={} resource=0x{:x} serial={}",
                e.error_code,
                e.request_code,
                e.minor_code,
                e.resourceid,
                e.serial
            );
        } else {
            log::warn!("[x11] suppressed X protocol error (null event)");
        }
        0
    }

    static INIT: Once = Once::new();
    INIT.call_once(|| {
        unsafe {
            XSetErrorHandler(Some(silent_handler));
        }
        log::info!(
            "[x11] installed silent X error handler to prevent BadWindow exits on Wayland-XWayland (issue #2001 item #2)"
        );
    });
}

#[cfg(not(target_os = "linux"))]
fn install_silent_x_error_handler() {}

pub fn run() {
    // Neutralise a broken inherited stderr *pipe* BEFORE any `eprintln!` can
    // fire. On Windows, when the GUI process inherits an stderr pipe whose
    // parent end later closes, the next stdlib stderr write fails with a
    // broken-pipe errno and `std::io::stdio::print_to` raises
    // `panic!("failed printing to stderr: …")` on the main thread, aborting the
    // app over an external condition (Sentry TAURI-RUST-F). Redirecting that
    // pipe to NUL makes the write succeed (discarded) instead of panicking. A
    // panic hook cannot fix this — it runs during unwinding and cannot stop the
    // abort — so the cure is at the write path. No-op on non-Windows / when
    // stderr is a console or file. See `stderr_panic_hook`.
    stderr_panic_hook::neutralize_broken_parent_stderr();

    // Must run before any GTK/CEF code that could trigger X calls — otherwise
    // Xlib's default handler calls exit(1) on the first BadWindow and we never
    // reach this line. See helper doc above for the full reasoning.
    install_silent_x_error_handler();

    // ── Install a custom tokio runtime for tauri::async_runtime ─────────
    //
    // Tauri's default async runtime uses tokio multi-thread workers with
    // a ~2 MB stack. The in-process core (spawned by
    // `core_process::CoreProcessHandle::ensure_running` via
    // `tokio::spawn(run_server_embedded(..))`) runs *on* that runtime, so
    // every JSON-RPC handler — including the deep tower
    // `web channel chat → orchestrator turn → delegate_to_integrations_agent
    // → sub-agent → composio_list_tools → load_config_with_timeout` —
    // burns through the same 2 MB. In `crahs.log` (2026-05-17, build
    // 0.53.49) that tower plus the serde-monomorphised `Config` Visitor
    // frames pushed past the guard page and aborted with
    // `SIGBUS / KERN_PROTECTION_FAILURE`.
    //
    // The structural fix (`spawn_blocking` for the TOML parse + cache in
    // `src/openhuman/config/{schema/load.rs, ops.rs}`) moves the largest
    // contributor off the worker. An initial 8 MiB bump shipped here was
    // enough for that single tower, but sub-agent delegation (issue #3159
    // / PR #3155) re-tipped the scale: the standalone `openhuman-core`
    // CLI server still aborted with `Abort trap: 6 / fatal runtime error:
    // stack overflow` once an orchestrator delegated. PR #3155 raised the
    // standalone server to 16 MiB; the desktop Tauri host is the *same*
    // tower running on a *different* runtime and needs the same headroom.
    // Share the constant with the rest of `src/core/*` via
    // [`openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES`] so all
    // multi-thread runtimes that may host an agent turn stay in sync.
    //
    // Must happen before any `tauri::async_runtime::*` call, otherwise
    // `set(...)` panics with "runtime already initialized".
    {
        let custom_runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
            .max_blocking_threads(openhuman_core::core::runtime::MAX_BLOCKING_THREADS)
            .build()
            .expect("build custom tokio runtime for tauri async surface");
        let handle = custom_runtime.handle().clone();
        // Tauri docs: "you cannot drop the underlying TokioRuntime."
        // Leak it so its lifetime matches the process.
        std::mem::forget(custom_runtime);
        tauri::async_runtime::set(handle);
    }

    // Initialize Sentry for the Tauri shell (desktop host) process before any
    // other startup work. Reads `OPENHUMAN_TAURI_SENTRY_DSN` at runtime first,
    // then falls back to the value baked in at compile time via the release
    // workflow. Missing/empty DSN ⇒ `sentry::init` returns a no-op guard.
    //
    // The guard is held for the entire lifetime of `run()` so events queued
    // during shutdown still flush. Only invoked here (and not in `main.rs`)
    // so renderer/GPU CEF helper subprocesses (re-exec'd via
    // `tauri::cef_entry_point`) and the `OpenHuman core …` in-process core
    // path do NOT spin up a second client — those have their own reporting
    // surfaces.
    let _sentry_guard = sentry::init(sentry::ClientOptions {
        dsn: std::env::var("OPENHUMAN_TAURI_SENTRY_DSN")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| option_env!("OPENHUMAN_TAURI_SENTRY_DSN").map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse().ok()),
        release: Some(std::borrow::Cow::Owned(build_sentry_release_tag())),
        environment: Some(std::borrow::Cow::Owned(resolve_sentry_environment())),
        send_default_pii: false,
        before_send: Some(std::sync::Arc::new(|mut event| {
            // Drop "dev-server fetch failed" noise: the vendored
            // `tauri-runtime-cef` dev proxy
            // (vendor/tauri-cef/crates/tauri/src/protocol/tauri.rs) calls
            // `log::error!("Failed to request {url}: {err}")` whenever the
            // CEF webview asks for an asset on `http://localhost:1420` (the
            // Vite dev URL baked into `tauri.conf.json`). That `log::error!`
            // is bridged into `tracing` and picked up by the sentry-tracing
            // layer as an Event — see `src/core/logging.rs::sentry_tracing_layer`.
            // In packaged staging/production builds Vite isn't running, so
            // the request correctly fails — but the failure is noise we
            // don't want in Sentry (issue OPENHUMAN-TAURI-V, 66+ events).
            // See [sentry-localhost-filter] log line below for diagnostics.
            if event_is_localhost_dev_fetch_noise(&event) {
                log::debug!(
                    "[sentry-localhost-filter] dropping dev-server fetch noise event: {:?}",
                    event.message.as_deref().unwrap_or("<no message>")
                );
                return None;
            }
            if openhuman_core::core::observability::is_budget_event(&event) {
                // Log only structured tag metadata — `event.message` can carry
                // upstream provider error text including tokens / pasted-through
                // secrets, and per `CLAUDE.md` "never log secrets or full PII".
                // The (domain, status) pair is sufficient diagnostic since
                // those are the tags `is_budget_event` gates on.
                log::debug!(
                    "[sentry-budget-filter] dropping budget-exhausted event (domain={:?}, status={:?})",
                    event.tags.get("domain"),
                    event.tags.get("status")
                );
                return None;
            }
            // Defense-in-depth: drop max-tool-iterations cap events that
            // slipped past the call-site filters in the core (see
            // `openhuman_core::core::observability::is_max_iterations_event`
            // for the rationale). The shell links the core in-process so
            // any captured event for this deterministic agent-state
            // outcome is filtered here too (OPENHUMAN-TAURI-99 / -98).
            if openhuman_core::core::observability::is_max_iterations_event(&event) {
                log::debug!(
                    "[sentry-max-iter-filter] dropping max-iteration cap noise event: {:?}",
                    event.message.as_deref().unwrap_or("<no message>")
                );
                return None;
            }
            if openhuman_core::core::observability::is_transient_backend_api_failure(&event)
                || openhuman_core::core::observability::is_transient_integrations_failure(&event)
                || openhuman_core::core::observability::is_updater_transient_event(&event)
                || openhuman_core::core::observability::is_skill_install_user_fetch_failure(&event)
            {
                return None;
            }
            // Defense-in-depth: drop managed-backend `errorCode` events (#870)
            // the backend owns (F2/F4). The shell links the core in-process,
            // so a managed inference error captured here must be filtered
            // identically to the core binary's main.rs chain. The malformed
            // `BAD_REQUEST` carve-out (F8) is excluded by the underlying
            // decision, so a client-built bad payload still pages.
            if openhuman_core::core::observability::is_backend_error_code_event(&event) {
                log::debug!(
                    "[sentry-error-code-filter] dropping backend-owned errorCode event_id={:?}",
                    event.event_id
                );
                return None;
            }
            // Defense-in-depth: drop transient streaming transport blips
            // (domain=llm_provider, failure=transport) — flaky-network
            // timeouts/resets recovered by retry/fallback (F7). Mirrors the
            // core binary's main.rs filter.
            if openhuman_core::core::observability::is_transient_provider_transport_failure(&event)
            {
                log::debug!(
                    "[sentry-transport-filter] dropping transient provider transport event_id={:?}",
                    event.event_id
                );
                return None;
            }
            // Drop 401 "Session expired. Please log in again." bodies and
            // pre-flight "no session token stored" guards — mirrors the
            // core binary's before_send chain. Since #1061 the Tauri shell
            // links the core in-process, so any session-expired event
            // captured by either surface lands in the same Sentry client
            // here and must be filtered identically. Keeps
            // OPENHUMAN-TAURI-25 / -1Q / -27 / -1G off Sentry.
            if openhuman_core::core::observability::is_session_expired_event(&event) {
                // Metadata-only log shape — `event.message` carries the raw
                // backend response body which CLAUDE.md forbids from local
                // logs. Mirror the core binary's main.rs filter.
                log::debug!(
                    "[sentry-session-expired-filter] dropping session-expired event_id={:?}",
                    event.event_id
                );
                return None;
            }
            // Drop provider insufficient-credits 402s — the user's own BYO
            // account (e.g. OpenRouter) is out of balance, a billing state
            // OpenHuman has no lever over once the request already caps
            // max_tokens. The core binary's main.rs before_send already
            // filters these; since #1061 the core runs in-process inside this
            // shell, so the cron `agent_job` retries-exhausted report (and any
            // other compatible-provider path) lands in THIS Sentry client and
            // must be filtered identically. Closes the #3617 drift that wired
            // the filter only into the standalone-CLI chain (TAURI-RUST-514 /
            // -C62).
            if openhuman_core::core::observability::is_insufficient_credits_event(&event) {
                // Metadata-only log shape — `event.message` carries the raw
                // provider 402 body which CLAUDE.md forbids from local logs.
                log::debug!(
                    "[sentry-insufficient-credits-filter] dropping insufficient-credits 402 event_id={:?}",
                    event.event_id
                );
                return None;
            }
            // Drop provider monthly-quota exhausted events — the user's
            // third-party plan has spent its allotment (e.g. Kiro
            // `MONTHLY_REQUEST_COUNT`, sometimes wrapped in a 500 envelope so
            // the 402-gated credits filter above misses it). No local lever;
            // mirrors the core binary's main.rs before_send chain
            // (TAURI-RUST-C9A: 9k events from a single quota-capped user).
            if openhuman_core::core::observability::is_quota_exhausted_event(&event) {
                // Metadata-only log shape — `event.message` carries the raw
                // provider body which CLAUDE.md forbids from local logs.
                log::debug!(
                    "[sentry-quota-exhausted-filter] dropping monthly-quota event_id={:?}",
                    event.event_id
                );
                return None;
            }
            // Defense-in-depth: drop Windows `ERROR_FILE_SYSTEM_LIMITATION`
            // (os error 665) — a persistent host-filesystem condition with
            // zero local lever and no Sentry remediation path. The Tauri
            // shell is a separate crate from the core, so the core's emit-site
            // classifier (`expected_error_kind`) can only catch events that
            // originate inside the core binary. Any filesystem-error event
            // that starts in the shell (e.g. file_logging, window_state,
            // CEF profile I/O) bypasses the core classifier and lands here;
            // this filter is the only net for those events (TAURI-RUST-QT0:
            // 6,050 events / 1 user).
            if openhuman_core::core::observability::is_windows_file_system_limitation_event(&event)
            {
                log::debug!(
                    "[sentry-fs-limitation-filter] dropping Windows file-system-limitation event (os error 665) event_id={:?}",
                    event.event_id
                );
                return None;
            }
            // Strip server_name (hostname) to avoid leaking machine identity.
            event.server_name = None;
            // Attach the cached account uid so Sentry can count unique users
            // affected by an issue. We only carry `id` — never email, name,
            // or IP — so this stays consistent with `send_default_pii: false`.
            // Since #1061 the core runs in-process inside this shell, so this
            // is the surface that tags ~all desktop events.
            //
            // Issue #3135: the primary source for `event.user` is now the
            // Sentry scope, bound proactively at session boundaries
            // (credentials::store_session / clear_session) and at server boot
            // (run_server_inner). The `app_state_snapshot` cache is kept as a
            // fallback for legacy cache-warming paths, but we only consult it
            // when the scope hasn't already bound a user — otherwise we'd
            // silently clobber the scope binding when the cache is empty
            // (the original userCount=0 root cause).
            if event.user.is_none() {
                event.user =
                    openhuman_core::openhuman::desktop::app_state::peek_cached_current_user_identity()
                        .and_then(|identity| identity.id)
                        .map(|id| sentry::User {
                            id: Some(id),
                            ..Default::default()
                        });
            }
            Some(event)
        })),
        sample_rate: 1.0,
        transport: Some(std::sync::Arc::new(
            openhuman_core::core::sentry_transport::factory,
        )),
        ..sentry::ClientOptions::default()
    });
    // Tag every Sentry event with CPU architecture and OS so Intel-specific
    // crashes (issue #1012 — SIGABRT in CrBrowserMain on x86_64 macOS) are
    // clearly identified without needing a separate build identifier.
    sentry::configure_scope(|scope| {
        scope.set_tag("cpu_arch", std::env::consts::ARCH);
        scope.set_tag("os_name", std::env::consts::OS);
        #[cfg(target_os = "macos")]
        if let Some(ver) = macos_os_version() {
            scope.set_tag("os_version", ver);
        }
    });

    // Install the panic hook *after* `sentry::init` so `take_hook()` captures
    // Sentry's panic integration as its chain target. The hook ALWAYS chains to
    // the previous hook for every panic — it never swallows, so no crash is ever
    // hidden from Sentry. The actual broken-pipe-on-stderr abort is prevented at
    // the write path by `neutralize_broken_parent_stderr()` (called at the top of
    // `run()`); this hook only adds a diagnostic breadcrumb for that family. See
    // `stderr_panic_hook` for the full rationale (Sentry TAURI-RUST-F).
    stderr_panic_hook::install();

    // Optional smoke trigger for verifying the Sentry pipeline end-to-end.
    // Run with `OPENHUMAN_TAURI_SENTRY_TEST=panic` to fire a panic, or
    // `=message` to send a captured-message event. No-op when unset.
    if let Ok(mode) = std::env::var("OPENHUMAN_TAURI_SENTRY_TEST") {
        match mode.as_str() {
            "panic" => panic!("OPENHUMAN_TAURI_SENTRY_TEST=panic — local Sentry smoke test"),
            "message" => {
                sentry::capture_message(
                    "OPENHUMAN_TAURI_SENTRY_TEST=message — local Sentry smoke test",
                    sentry::Level::Error,
                );
                let _ = sentry::Hub::current().client().map(|c| c.flush(None));
            }
            other => log::warn!(
                "OPENHUMAN_TAURI_SENTRY_TEST={other:?} — unknown mode (use 'panic' or 'message')"
            ),
        }
    }

    let daemon_mode = is_daemon_mode();

    // Install the unified tracing subscriber + daily-rotated file appender
    // before any other startup work so CEF preflight failures, sentry
    // smoke-test events, and the rest of `run()` are captured in
    // `<data_dir>/logs/openhuman-YYYY-MM-DD.log`. The shell's `log::*` calls
    // are bridged into the same subscriber via `tracing_log::LogTracer`,
    // replacing the previous stderr-only `env_logger`.
    file_logging::init();

    // Log platform identity early so every log session is tagged with arch
    // and OS version — essential for reproducing and triaging Intel-only
    // crashes like issue #1012 (SIGABRT in CrBrowserMain on x86_64 macOS).
    {
        let arch = std::env::consts::ARCH;
        let os = std::env::consts::OS;
        #[cfg(target_os = "macos")]
        let os_ver = macos_os_version().unwrap_or_else(|| "unknown".to_string());
        #[cfg(not(target_os = "macos"))]
        let os_ver = "n/a".to_string();
        log::info!("[startup] platform: arch={arch} os={os} os_version={os_ver}");
    }

    warn_if_wsl_x11_desktop_launch();
    // Exit before CEF if no display server is available — prevents the
    // `assert_eq!(cef_initialize(…), 1)` panic (OPENHUMAN-TAURI-K1).
    check_linux_display_server();

    // The vendored tauri-cef dev-server proxy builds a reqwest 0.13 client
    // (see vendor/tauri-cef/crates/tauri/src/protocol/tauri.rs) which calls
    // rustls 0.23's `CryptoProvider::get_default()`. rustls 0.23 no longer
    // picks a provider implicitly — without one installed, the proxy panics
    // with "No provider set" the first time `tauri dev` forwards a request.
    // Install the ring provider once before any HTTPS client is built.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // ── Windows pre-CEF single-instance guard (Sentry OPENHUMAN-TAURI-A) ──
    //
    // `tauri_plugin_single_instance` detects a second launch inside its
    // `.setup()` hook — but `.setup()` runs AFTER `Builder::build()` which
    // calls `CefRuntime::init` → `cef::initialize()`. On a second launch,
    // `cef::initialize()` returns 0 because the primary holds the CEF
    // cache lock; the vendored runtime asserts `result == 1` and panics
    // (left: 0, right: 1, fatal, Windows-only, 598 events).
    //
    // Fix: acquire a named Win32 mutex at the very top of `run()` — before
    // any CEF or builder work — so any secondary instance sees
    // `ERROR_ALREADY_EXISTS` and exits immediately. If the secondary was
    // launched for an `openhuman://` OAuth callback, forward that URL to the
    // primary through our pre-CEF pipe before exiting; the Tauri deep-link
    // plugin cannot run on this early secondary path.
    //
    // The RAII guard holds the mutex handle for the lifetime of `run()`.
    // Windows releases all process handles automatically on exit, so
    // explicit cleanup is only needed if `run()` returns normally.
    #[cfg(windows)]
    let _cef_init_mutex_guard = {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
        use windows_sys::Win32::System::Threading::CreateMutexW;

        // Must match the bundle identifier in tauri.conf.json.
        // Changing the app identifier requires updating this string too.
        let mutex_name: Vec<u16> = "com.openhuman.app-cef-init\0".encode_utf16().collect();

        // SAFETY: mutex_name is null-terminated UTF-16; handle is checked below.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
        // Capture GetLastError immediately after CreateMutexW so no intervening
        // syscall (e.g. logging) can clobber the thread-local error code.
        let last_error = unsafe { GetLastError() };

        // Primary: hold the handle until run() returns.
        struct OwnedMutex(isize);
        impl Drop for OwnedMutex {
            fn drop(&mut self) {
                if self.0 != 0 {
                    unsafe { CloseHandle(self.0 as _) };
                }
            }
        }

        if handle.is_null() {
            // CreateMutexW failed for a reason other than "already exists"
            // (which returns a valid handle plus ERROR_ALREADY_EXISTS). Likely
            // causes: out-of-memory, security-descriptor fault, or other
            // Win32-level anomaly. Without the guard, a concurrent second
            // launch can re-trigger the cef::initialize panic this block was
            // added to prevent — but refusing to start at all is strictly
            // worse for the user. Log loudly so the failure is observable in
            // Sentry / log files and continue best-effort.
            log::error!(
                "[single-instance] CreateMutexW returned NULL handle (GetLastError={last_error}); continuing without pre-CEF single-instance guard — concurrent launches may hit OPENHUMAN-TAURI-A"
            );
            OwnedMutex(0)
        } else if last_error == ERROR_ALREADY_EXISTS {
            // Another instance is already past this point — exit before we
            // touch CEF at all. Forward deep links first so OAuth callbacks
            // are not dropped by this early pre-plugin exit.
            match deep_link_ipc_windows::try_forward_deep_links() {
                deep_link_ipc_windows::ForwardResult::Forwarded
                | deep_link_ipc_windows::ForwardResult::NoUrls => {}
                deep_link_ipc_windows::ForwardResult::NoPrimary => {
                    log::warn!(
                        "[single-instance] secondary had deep-link argv but could not reach primary pipe"
                    );
                }
            }
            unsafe { CloseHandle(handle) };
            log::info!(
                "[single-instance] pre-CEF mutex held by primary; secondary exiting (OPENHUMAN-TAURI-A fix)"
            );
            std::process::exit(0);
        } else {
            OwnedMutex(handle as isize)
        }
    };

    #[cfg(windows)]
    let _deep_link_pipe_guard = deep_link_ipc_windows::bind_and_listen();

    // ── Windows proactive stale-process reap (issues #3605, #3900) ─────────
    // The Win32 mutex above already guaranteed we are the only *top-level*
    // GUI instance past that point — a concurrent secondary saw
    // `ERROR_ALREADY_EXISTS` and exited before reaching here. So a surviving
    // GUI `OpenHuman.exe` browser process is a *wedged prior instance* left
    // behind by an update or hard exit, not a legitimate peer. Reap it
    // proactively (TERM then KILL) — the cross-platform analogue of the macOS
    // reap above.
    //
    // The reap is deliberately narrow (issue #3900): it targets ONLY the
    // wedged GUI browser process. It never touches `OpenHuman.exe core` /
    // `mcp` CLI/MCP sessions or the standalone `openhuman-core.exe` (which
    // never take the CEF mutex and may be an active user session), never
    // touches CEF `--type=` helper subprocesses (the OS job object reaps
    // those with their parent), never touches this process's ancestors (the
    // old app that spawned us during an update relaunch), and force-kills
    // without `/T` so a tree walk can never reach the freshly launched app.
    //
    // This must run BEFORE `cef_singleton_wait::wait_for_cache_release()`:
    // a wedged prior process that never exits on its own keeps the CEF
    // cache lock until that bounded wait times out and exits this instance
    // (code 0) — i.e. the updated app silently refuses to start until the
    // user manually kills the old process (the exact #3605 symptom).
    // Actively reaping the holder lets the new version take over instead of
    // bailing out.
    //
    // NOT done on Linux here: unlike Windows, the Linux
    // `tauri-plugin-single-instance` guard registers later (in the builder),
    // so at this preflight point a concurrent second launch has no
    // single-instance guarantee and a bare reap could kill a legitimate
    // peer. Linux needs the macOS-style live-lock-holder guard first.
    #[cfg(target_os = "windows")]
    process_recovery::reap_stale_openhuman_processes();

    // ── Windows pre-CEF cache-lock wait (Sentry TAURI-RUST-F) ─────────────
    // The Win32 mutex above stops a *concurrent* second launch, but on a
    // *sequential* relaunch (auto-update, fast quit+reopen, restart) the prior
    // instance can still hold the CEF cache lock for a short teardown window
    // after releasing the mutex. Calling `cef::initialize()` into that live
    // lock returns 0 and the vendored runtime asserts `== 1` → panic. Wait
    // (bounded) for the prior instance to exit before proceeding; this never
    // suppresses a crash — it prevents `cef::initialize()` from running
    // against a locked cache. Analogous to the macOS reap above.

    // ── Linux pre-CEF deep-link forwarding guard (issue #2359) ────────────
    // On Linux, a secondary instance with an openhuman:// URL in argv exits
    // at the CEF preflight check before Builder::setup() runs, silently
    // dropping the OAuth callback. Detect and forward the URL here, before
    // CEF preflight can exit(1).
    #[cfg(target_os = "linux")]
    let _deep_link_socket_guard = {
        use deep_link_ipc::ForwardResult;
        match deep_link_ipc::try_forward_deep_links() {
            ForwardResult::Forwarded => {
                std::process::exit(0);
            }
            ForwardResult::NoPrimary | ForwardResult::NoUrls => {}
        }
        deep_link_ipc::bind_and_listen()
    };

    // CEF cache-lock preflight (macOS + Linux): if another OpenHuman instance
    // holds the CEF user-data-dir SingletonLock, `cef_initialize` returns 0 and
    // the vendored runtime used to panic (`left: 0, right: 1`). The common
    // cause is a *sequential relaunch race* where the prior instance is still
    // tearing down, so rather than exit on the first collision we wait
    // (bounded, exponential backoff — the macOS/Linux analogue of the Windows
    // pre-CEF wait) for the lock to clear, then proceed. If it is still held
    // after the budget we exit cleanly (code 0). Stale locks (PID dead) are
    // removed so crashed processes don't block launches. macOS: issue #864.
    // Linux: OPENHUMAN-TAURI-K1. Sentry: TAURI-RUST-F.
    //
    // ── macOS/Linux pre-CEF stale-lock reap (issue #4395) ─────────────────
    // Before the bounded wait above can only *time out* on a wedged prior
    // instance (the #3605 symptom: the updated app silently refuses to start),
    // proactively reap that instance — but ONLY when a recent post-update
    // relaunch marker proves the CEF-lock holder is the pre-update process,
    // never a healthy running one. This is the macOS/Linux analogue of the
    // Windows pre-CEF reap, gated on a staleness signal because these targets
    // have no early single-instance mutex. See `cef_stale_reap` for the safety
    // rationale (and why the reverted #3793 SIGKILL is not reintroduced).

    let builder = {
        #[cfg(feature = "e2e-test-support")]
        {
            // Bypass macOS Keychain. Without this, every embedded service that
            // touches password / cookie / encryption-key storage triggers a
            // "Allow access to your keychain?" prompt — WhatsApp Web hits it on
            // every cold start, Chromium's own component-update store also does.
            // `use-mock-keychain` swaps the Keychain backend for an in-process
            // mock; `password-store=basic` is the equivalent for the password
            // manager. Both are no-ops on Windows/Linux, so safe to always set.
            //
            // No DevTools/CDP port is opened in any build. The CDP layer and
            // the Appium `debuggerAddress` harness that used it were removed in
            // #5478 — CDP does not exist under the Wry runtime.
            //
            // NOTE: flags must be prefixed with `--`. The runtime's
            // `on_before_command_line_processing` dispatch (in
            // `tauri-runtime-cef/src/cef_impl.rs`) routes value-less args that
            // don't start with `-` to `append_argument` (positional) instead of
            // `append_switch`, which means Chromium silently ignores them.
            let mut args: Vec<CefCommandLineArg> = vec![
                ("--use-mock-keychain", None),
                ("--password-store", Some("basic")),
                // Enable SharedArrayBuffer so embedded apps that need WebRTC
                // audio worklets / Opus encoders (Slack Huddles, Meet
                // real-time features, Discord voice) can actually initialise.
                // Chromium gates SharedArrayBuffer behind cross-origin
                // isolation by default; web apps embedded inside CEF rarely
                // send COOP/COEP headers, so without this flag the feature
                // silently disappears and huddle/call buttons no-op.
                ("--enable-features", Some("SharedArrayBuffer")),
                // Defeat Chromium's modern throttlers that ignore the
                // legacy `--disable-background-timer-throttling` flag.
                // Empirically with that flag *alone*, the producer in the
                // (visible but non-key) main window still got pinned to
                // 1Hz worker timers as soon as the off-screen Meet window
                // opened. These three feature flags are the canonical
                // additional knobs (Electron / Puppeteer use them).
                (
                    "--disable-features",
                    Some(
                        "IntensiveWakeUpThrottling,CalculateNativeWinOcclusion,\
                     AutofillActorMode,GlicActorUi,LensOverlay",
                    ),
                ),
                // Allow autoplay (audio + video) without a prior user gesture.
                // CEF inherits Chromium's default policy, which leaves an
                // AudioContext suspended until the user interacts with the
                // page; @remotion/player gates its rAF frame loop on
                // AudioContext.state === 'running', so on a cold tab the
                // mascot SVG paints frame 0 and freezes there until the user
                // alt-tabs / clicks somewhere (which counts as a gesture and
                // resumes the context — the "fast on revisit" symptom). With
                // this flag the AudioContext starts in 'running' immediately
                // and the mascot animates from first paint. We control every
                // surface in this webview, so dropping the gesture gate is
                // safe.
                ("--autoplay-policy", Some("no-user-gesture-required")),
                // Background-throttling defeaters. The MeetCallProducer
                // pumps mascot frames at 24 fps from the *main* OpenHuman
                // window, but as soon as the off-screen Meet webview opens
                // (or the user clicks anywhere outside main), macOS demotes
                // the renderer's priority and Chromium throttles its
                // setInterval / worker timers / rAF down to ~1 Hz — the
                // mascot stream collapses to 1 fps. Page-level workarounds
                // (silent AudioContext, muted <audio>) are unreliable in
                // CEF: AudioContext starts suspended pre-gesture; the muted
                // <audio> trick depends on the renderer being polled at all.
                // The canonical fix is the Chromium command-line flag set
                // Electron / Puppeteer use for the same scenario.
                //
                // Process-wide is fine: every CEF webview we own is part of
                // the agent flow (no idle low-priority background tabs we
                // care about saving battery on).
                ("--disable-background-timer-throttling", None),
                ("--disable-renderer-backgrounding", None),
                ("--disable-backgrounding-occluded-windows", None),
            ];
            let force_gpu_env = std::env::var("OPENHUMAN_FORCE_GPU").ok();
            let disable_gpu_env = std::env::var("OPENHUMAN_DISABLE_GPU").ok();
            append_platform_cef_gpu_workarounds(
                &mut args,
                std::env::consts::OS,
                std::env::consts::ARCH,
                force_gpu_env.as_deref(),
                disable_gpu_env.as_deref(),
            );
            // #3554: never forward a `--time-ticks-at-unix-epoch` switch to CEF —
            // a corrupt/negative value drives the renderer's clock decades off and
            // shows a wrong "Current Date & Time". Let Chromium compute it.
            strip_time_ticks_at_unix_epoch(&mut args);
            let _ = args;
            tauri::Builder::<AppRuntime>::new()
        }

        #[cfg(not(feature = "e2e-test-support"))]
        tauri::Builder::<AppRuntime>::new()
    };

    #[cfg(target_os = "macos")]
    let builder = builder
        // Use an app-owned Quit item for Cmd+Q instead of the native
        // predefined Quit action. The predefined path calls
        // NSApplication::terminate, which reaches CEF shutdown before
        // OpenHuman's child-webview/core teardown can run.
        .menu(macos_app_menu)
        .on_menu_event(|app, event| {
            if event.id().as_ref() == APP_QUIT_MENU_ID {
                log::info!("[app-menu] action=quit");
                shutdown_app_sync(app, 0);
            }
        });

    // Single-instance guard — MUST be the first plugin registered so the
    // secondary-process exit path triggers before any other plugin setup
    // (and before `Builder::build()` reaches `CefRuntime::init`). Without
    // this, launching a second instance races into CEF init while the
    // primary still holds the cache lock; `cef::initialize` returns 0 and
    // the vendored runtime asserts (Sentry OPENHUMAN-TAURI-A — 442 events
    // across Win10/11 + Linux, all releases). The callback receives the
    // secondary's argv/cwd; we forward deep-link args and focus the main
    // window. Deep-link payloads stay handled by `tauri-plugin-deep-link`
    // — we just need to wake the primary so it observes them.
    //
    // On Linux the plugin's `setup()` calls
    // `zbus::blocking::connection::Builder::session().unwrap()`, which panics
    // with `Address("unsupported transport 'disabled'")` when the D-Bus
    // session bus is unreachable (WSL2-without-WSLg, minimal containers, root
    // launches without `$XDG_RUNTIME_DIR/bus`) — Sentry OPENHUMAN-TAURI-TM.
    // Probe for a usable session bus first and skip the plugin if absent;
    // single-instance enforcement is best-effort in those environments
    // anyway, and a graceful skip is strictly better than crashing at
    // startup.
    let builder = if can_register_single_instance_plugin() {
        builder.plugin(tauri_plugin_single_instance::init(
            |app: &AppHandle<AppRuntime>, args, cwd| {
                // Don't log raw argv/cwd: deep-link callbacks (OAuth codes,
                // magic links) can carry auth tokens that would otherwise leak
                // into desktop logs and Sentry breadcrumbs. CodeRabbit on #1510.
                log::info!(
                    "[single-instance] secondary launch detected, focusing primary (argc={}, cwd_present={})",
                    args.len(),
                    !cwd.is_empty()
                );
                if let Err(err) = show_main_window(app) {
                    log::warn!("[single-instance] failed to focus main window: {err}");
                }
            },
        ))
    } else {
        log::warn!(
            "[single-instance] D-Bus session bus unreachable (DBUS_SESSION_BUS_ADDRESS={:?}, \
             XDG_RUNTIME_DIR={:?}); skipping tauri-plugin-single-instance to avoid \
             OPENHUMAN-TAURI-TM panic. Multiple OpenHuman instances will not be deduplicated.",
            std::env::var("DBUS_SESSION_BUS_ADDRESS").ok(),
            std::env::var("XDG_RUNTIME_DIR").ok()
        );
        builder
    };
    let builder = builder
        // Explicitly disable `open_js_links_on_click`: tauri-plugin-opener
        // defaults to injecting `init-iife.js` into *every* webview — a
        // global click listener that invokes `plugin:opener|open_url` via
        // HTTP-IPC. That violates our "no JS injection into child
        // webviews" rule (see CLAUDE.md) and also fails in practice
        // because third-party origins trip Tauri's Origin header check
        // and return 500. The main window uses `openUrl()` from
        // `utils/openUrl.ts` when it needs to hand off a URL.
        .plugin(
            tauri_plugin_opener::Builder::default()
                .open_js_links_on_click(false)
                .build(),
        )
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // Auto-updater for the Tauri shell. Endpoint and minisign pubkey live
        // in `tauri.conf.json` under `plugins.updater`. Releases are signed at
        // build time with `TAURI_SIGNING_PRIVATE_KEY` (+ `_PASSWORD`); see
        // gitbooks/overview/auto-update.md for the full pipeline.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(dictation_hotkeys::DictationHotkeyState(
            std::sync::Mutex::new(Vec::new()),
        ))
        .manage(ptt_hotkeys::PttHotkeyState::new())
        .manage(PendingAppUpdateState::default());
    let builder = builder.manage(std::sync::Arc::new(imessage_scanner::ScannerRegistry::new()));
    builder
        .setup(move |app| {
            #[cfg(windows)]
            {
                // `register_all` writes HKCU\Software\Classes\openhuman so the
                // browser can hand `openhuman://auth?...` callbacks back to
                // the running instance. The plugin only returns an Err — and
                // it only logs at `warn` — when its single internal write
                // fails outright; it does not verify what's on disk. Issue
                // #2699 reports OAuth callbacks silently disappearing on
                // some Windows installs, which traced back to a missing or
                // stale `command` value here. Read it back and log loudly
                // (Sentry-level `error`) so the failure mode is observable
                // in support logs; we deliberately do NOT auto-repair —
                // writing the wrong exe path can brick a working install.
                let register_err = app.deep_link().register_all().err();
                let status = deep_link_registration_check::verify_protocol_registration();
                let status_log = status.redacted();
                if register_err.is_none() && status.is_healthy() {
                    log::info!("[deep-link] openhuman:// scheme registered ({status_log})");
                } else {
                    // Use the redacted form so per-user install paths
                    // (`C:\Users\<username>\...`) do not land in Sentry / user
                    // logs — basenames are kept so the diagnostic still
                    // identifies the registered exe.
                    log::error!(
                        "[deep-link] openhuman:// scheme registration unhealthy — \
                         OAuth callbacks may never reach the app. \
                         register_all_error={register_err:?}, hkcu_status={status_log}. \
                         See gitbooks/overview/troubleshooting-sign-in.md \
                         (\"Windows: openhuman:// handler not registered\") for the manual repair."
                    );
                }
                deep_link_ipc_windows::drain_pending_urls(app.app_handle());
            }
            #[cfg(target_os = "linux")]
            {
                // `tauri-plugin-deep-link::register_all` on Linux shells out
                // to `xdg-mime`, `update-desktop-database`, and
                // `xdg-icon-resource` in sequence to install MIME-type
                // associations for our custom URL schemes. On Linux installs
                // that ship without one or more of those binaries — WSL2
                // without a desktop env, headless servers, minimal
                // containers (OPENHUMAN-TAURI-AS: WSL2 user in BR;
                // OPENHUMAN-TAURI-5V: same shape but `xdg-mime` was
                // installed while `update-desktop-database` was missing) —
                // the plugin fires
                // `log::error!("Failed to run OS command \`<name>\`…")`
                // *internally* before returning the Err. That internal
                // error log is scooped up by `sentry-tracing` into a Sentry
                // event even though our `if let Err` arm below already
                // demotes the user-visible failure to a warn.
                //
                // Pre-flight every binary the plugin will shell out to and
                // skip `register_all` entirely if any of them is missing —
                // partial registration can't succeed because the plugin
                // runs all three in sequence inside `register_all`, so the
                // first missing binary kills the whole flow. Registration
                // only matters on systems with a desktop environment,
                // where xdg-utils ships as a single package.
                const XDG_BINARIES: &[&str] =
                    &["xdg-mime", "update-desktop-database", "xdg-icon-resource"];
                let missing: Vec<&str> = XDG_BINARIES
                    .iter()
                    .copied()
                    .filter(|name| !path_has_executable(name))
                    .collect();
                if missing.is_empty() {
                    if let Err(err) = app.deep_link().register_all() {
                        log::warn!("[deep-link] register_all failed (non-fatal): {err}");
                    }
                } else {
                    log::warn!(
                        "[deep-link] skipping register_all — xdg-utils binaries missing on PATH: {} \
                         (xdg-utils not installed; deep-link MIME registration unavailable on this host)",
                        missing.join(", ")
                    );
                }

                // Drain any deep-link URLs that arrived via the IPC socket
                // before setup() ran (issue #2359). Also installs the live
                // handler so URLs arriving after setup() are emitted directly.
                deep_link_ipc::drain_pending_urls(app.app_handle());
            }

            // Purge stray LaunchAgent left over from a prior worktree's
            // `service install`. KeepAlive=true on the plist re-spawns the
            // daemon after every SIGKILL, fighting `ensure_running`'s
            // stale-listener takeover and re-binding port 7788 on cold boot.
            // (Symptom: "Failed to start local core: signaled pid <X> but
            // port 7788 remained bound after 5000ms".)
            //
            // Tightly scoped to avoid clobbering a legitimate `service
            // install`:
            //   - dev builds only (`cfg!(debug_assertions)`)
            //   - skip when this process IS the daemon (`!daemon_mode`)
            //   - only purge when the plist's ProgramArguments[0] points
            //     somewhere other than the currently-running executable —
            //     i.e. a sibling worktree's stale binary, not us.
            #[cfg(target_os = "macos")]
            if cfg!(debug_assertions) && !daemon_mode {
                const STALE_LABEL: &str = "com.openhuman.core";

                if let Ok(home) = std::env::var("HOME") {
                    let plist = std::path::PathBuf::from(&home)
                        .join("Library")
                        .join("LaunchAgents")
                        .join(format!("{STALE_LABEL}.plist"));

                    let plist_targets_us = std::fs::read_to_string(&plist)
                        .ok()
                        .and_then(|contents| {
                            // ProgramArguments[0] is the first <string>...</string>
                            // after the <key>ProgramArguments</key> marker. The
                            // service installer always writes it as an absolute
                            // path to the openhuman-core binary (see
                            // src/openhuman/platform/service/macos.rs).
                            let after_key = contents.split("<key>ProgramArguments</key>").nth(1)?;
                            let start = after_key.find("<string>")? + "<string>".len();
                            let rest = &after_key[start..];
                            let end = rest.find("</string>")?;
                            Some(std::path::PathBuf::from(rest[..end].trim()))
                        })
                        .zip(std::env::current_exe().ok())
                        .map(|(plist_bin, self_bin)| plist_bin == self_bin)
                        .unwrap_or(false);

                    if plist.exists() && !plist_targets_us {
                        let uid = std::process::Command::new("id")
                            .arg("-u")
                            .output()
                            .ok()
                            .and_then(|o| String::from_utf8(o.stdout).ok())
                            .map(|s| s.trim().to_string());

                        if let Some(uid) = uid {
                            let target = format!("gui/{uid}/{STALE_LABEL}");
                            let _ = std::process::Command::new("launchctl")
                                .arg("bootout")
                                .arg(&target)
                                .status();
                        }

                        match std::fs::remove_file(&plist) {
                            Ok(()) => log::warn!(
                                "[boot] removed stale LaunchAgent plist at {} \
                                 (points at a different binary than this build — \
                                 likely a sibling worktree's `service install`)",
                                plist.display()
                            ),
                            Err(err) => log::warn!(
                                "[boot] failed to remove stale LaunchAgent plist {}: {err}",
                                plist.display()
                            ),
                        }
                    }
                }
            }

            let core_handle =
                core_process::CoreProcessHandle::new(core_process::default_core_port());
            std::env::set_var("OPENHUMAN_CORE_RPC_URL", core_handle.rpc_url());

            // Cookie databases are implementation-private in the native
            // WebView runtime and are deliberately not exposed to the core.
            std::env::remove_var("OPENHUMAN_CEF_COOKIES_DB");

            app.manage(core_handle.clone());
            // NOTE: the core is NOT auto-spawned here. The BootCheckGate UI
            // calls `start_core_process` (Local mode) after the user picks a
            // mode, which lets the frontend surface startup failures and
            // version mismatches before the rest of the app mounts.
            //
            // In daemon mode (headless) we spawn immediately so the tray
            // agent is available without waiting for a UI interaction.
            if daemon_mode {
                let core_handle_daemon = core_handle.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(err) = core_handle_daemon.ensure_running().await {
                        log::error!("[core] daemon_mode — failed to start embedded core: {err}");
                        return;
                    }
                    log::info!("[core] daemon_mode — embedded core ready");
                });
            }

            // Restore last-known window position+size before showing the
            // window so the user's first paint after a restart-driven flow
            // (#900 identity flip) lands on the same display they used,
            // not back at the default centered initial size on the
            // primary monitor. `tauri.conf.json` ships `visible: false`
            // / `center: false` for the main window so the placement
            // happens before the first paint and there's no jump.
            if let Some(window) = app.get_webview_window("main") {
                // Layout first: mixed-DPI placement bugs (#5041) are not
                // diagnosable from a user report without it, and logging
                // before placement captures the pre-clamp state.
                window_state::log_monitor_layout(&window);
                // Installed before placement so a scale change triggered
                // by our own cross-monitor move is caught too — on
                // Windows that arrives via the message loop after
                // `setup()` returns, which is why clamping here alone is
                // not enough.
                window_state::install_dpi_guard(&window);
                // No saved geometry (first launch, or the save is stale /
                // belongs to a detached monitor) → open filling the work area
                // rather than the modest default size from `tauri.conf.json`.
                // `center_main` stays as the fallback for the case where no
                // monitor resolves at all.
                if !window_state::restore_main(&window)
                    && !window_state::maximize_to_work_area(&window)
                {
                    window_state::center_main(&window);
                }
                if !daemon_mode {
                    if let Err(err) = window.show() {
                        log::warn!("[window-state] show main window failed: {err}");
                    }
                    // CEF keyboard routing fix — cold launch:
                    //
                    // `window.show()` does not wire the renderer as the
                    // keyboard input target. `Window::set_focus` only
                    // dispatches `WindowMessage::SetFocus` → `request_focus`,
                    // which lifts the OS window but does not call
                    // `CefBrowserHost::SetFocus(true)`. Without that
                    // CEF-level focus, the textarea accepts focus on cold
                    // launch (cursor blinks) but `WM_KEYDOWN` messages
                    // never reach the renderer — typing is silently dead
                    // until the user click-outside / click-back triggers
                    // `WM_KILLFOCUS`+`WM_SETFOCUS`, which CEF's window
                    // handler routes through `host.set_focus(1)` internally.
                    //
                    // We need to call `webview.set_focus()` (which dispatches
                    // `WebviewMessage::SetFocus` → `host.set_focus(1)`)
                    // *after* CEF has finished creating the browser — too
                    // early and `browser()`/`host()` return None and the
                    // call silently no-ops. Defer the call to a spawned
                    // task with a small delay so CEF's browser-create
                    // settles. Then call it again after another delay as
                    // belt-and-suspenders for slower init paths.
                    // Previous attempts at calling `webview.set_focus()` alone
                    // confirmed the dispatch reaches CEF (both returned Ok),
                    // but keyboard routing stayed broken. `host.set_focus(1)`
                    // alone is insufficient — CEF's internal focus state
                    // needs a blur-then-focus *cycle* to wire keyboard
                    // routing on cold launch (matches the user-discovered
                    // workaround: click outside the window, then click back).
                    //
                    // The vendored tauri-cef doesn't expose `set_focus(false)`,
                    // so we mimic the cycle at the OS-window level:
                    // minimize triggers `WM_KILLFOCUS` (CEF's window handler
                    // propagates this to `host.set_focus(0)`), unminimize
                    // restores the window and triggers `WM_SETFOCUS` →
                    // `host.set_focus(1)`. Pair with explicit `set_focus`
                    // calls on both Window and Webview to cover the case
                    // where minimize/unminimize raced ahead of CEF's
                    // browser-create.
                    // Windows-only: the bug class (CEF host-renderer focus
                    // desync after a `visible: false` → `show()` transition
                    // without a real `WM_KILLFOCUS`+`WM_SETFOCUS` edge)
                    // manifests on the Windows CEF integration. macOS and
                    // Linux CEF use different focus propagation paths and
                    // don't exhibit the symptom, so running the
                    // minimize/unminimize cycle there would just be a
                    // visible flicker for no benefit. (Per CodeRabbit
                    // review on PR #1528.)
                    #[cfg(target_os = "windows")]
                    {
                    log::info!("[focus-fix] scheduling deferred CEF focus-cycle");
                    let webview_window_clone = window.clone();
                    tauri::async_runtime::spawn(async move {
                        // Wait for CEF to finish creating the browser host
                        // (synchronous setup() returns before this completes).
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        // Blur-then-focus cycle via minimize/unminimize.
                        // This is what the manual click-outside / click-back
                        // workaround does at the Win32 level.
                        log::info!("[focus-fix] starting minimize→unminimize focus cycle");
                        if let Err(err) = webview_window_clone.minimize() {
                            log::warn!("[focus-fix] minimize failed: {err}");
                        }
                        // Tiny pause so Windows actually processes the
                        // minimize before we ask to restore.
                        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                        if let Err(err) = webview_window_clone.unminimize() {
                            log::warn!("[focus-fix] unminimize failed: {err}");
                        }
                        // Belt-and-suspenders: explicit Window + Webview focus
                        // after the cycle in case the minimize→restore path
                        // didn't propagate.
                        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                        if let Err(err) = webview_window_clone.set_focus() {
                            log::warn!("[focus-fix] post-cycle window.set_focus failed: {err}");
                        }
                        let webview: &tauri::Webview<AppRuntime> = webview_window_clone.as_ref();
                        if let Err(err) = webview.set_focus() {
                            log::warn!("[focus-fix] post-cycle webview.set_focus failed: {err}");
                        }
                        log::info!("[focus-fix] focus cycle complete");
                    });
                    }
                }
            }

            if daemon_mode {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                    log::info!("[tray] daemon_mode=true window_hidden_on_startup");
                }
            }

            // Overlay window is currently disabled in `tauri.conf.json` (the
            // `overlay` entry under `app.windows` was removed), so we skip
            // the macOS NSPanel reclass + bottom-right pin + initial show
            // here. The helpers (`configure_overlay_window_macos`,
            // `pin_overlay_bottom_right`) and the React entry point
            // (`src/overlay/OverlayApp.tsx`) are kept intact so the overlay
            // can be re-enabled by restoring the config entry and the two
            // setup blocks below.
            //
            //   #[cfg(target_os = "macos")]
            //   if let Some(window) = app.get_webview_window("overlay") {
            //       configure_overlay_window_macos(&window);
            //   }
            //   if let Some(window) = app.get_webview_window("overlay") {
            //       pin_overlay_bottom_right(&window);
            //       let _ = window.show();
            //   }

            // Notch activity indicator: transparent pill at the top-centre of
            // the primary screen. Shows live voice / agent state. macOS only
            // (physical notch or menu-bar HUD on older hardware).
            //
            // It is NOT auto-shown here: the notch is the always-on listening
            // HUD, so its visibility is owned by the frontend, which calls
            // `notch_window_show` / `notch_window_hide` (via `syncNotchVisibility`)
            // to mirror `voice_server.always_on_enabled` — once on boot and
            // whenever the Settings toggle flips. Showing it unconditionally
            // here would flash the pill on every launch even with always-on
            // listening disabled (the default).

            // Tray icon setup moved to RunEvent::Ready (see below) — GTK is only
            // initialized after the event loop starts, so we must delay tray creation
            // until the Ready event fires. Creating the tray here would panic on
            // Linux with "GTK has not been initialized".
            log::info!("[tray] deferring tray setup to RunEvent::Ready");

            #[cfg(target_os = "macos")]
            {
                use std::sync::Arc;
                // The scanner task self-gates on `channels_config.imessage` via
                // JSON-RPC each tick — it stays idle until the user connects
                // iMessage and stops ingesting as soon as they disconnect. We
                // spawn it here just so the loop is live and picks up state
                // changes without requiring an app restart.
                if let Some(registry) = app.try_state::<Arc<imessage_scanner::ScannerRegistry>>() {
                    let registry = registry.inner().clone();
                    let app_handle = app.handle().clone();
                    registry.ensure_scanner(app_handle, "default".to_string());
                    log::info!("[imessage] scanner scheduled (gates on config each tick)");
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_rpc_url,
            core_rpc_token,
            core_rpc_endpoint,
            // Gateways: where the frontend's RPC goes. Absent, not stubbed,
            // when the feature is off.
            #[cfg(feature = "gateways")]
            gateway::commands::gateway_list,
            #[cfg(feature = "gateways")]
            gateway::commands::gateway_save,
            #[cfg(feature = "gateways")]
            gateway::commands::gateway_delete,
            #[cfg(feature = "gateways")]
            gateway::commands::gateway_activate,
            #[cfg(feature = "gateways")]
            gateway::commands::gateway_active,
            #[cfg(feature = "gateways")]
            gateway::commands::gateway_status,
            // Host-side HTTP relay so the renderer can reach a self-hosted
            // runtime on a LAN IP that the secure `tauri://localhost` webview
            // cannot fetch directly (cleartext mixed content). See #3865.
            core_rpc::relay_http_rpc,
            overlay_parent_rpc_url,
            process_diagnostics_list_owned,
            // Artifact export — cross-platform. Previously macOS/Linux-gated,
            // but the `directories` + `tokio::fs::copy` flow compiles on Windows
            // too (CodeRabbit on #4127). The Save-As dialog that used to sit in
            // front of this went with the shell's `rfd` dependency.
            artifact_commands::download_artifact_to_downloads,
            // Native directory chooser for the folder memory-source (#5831).
            // Unlike the Save-As dialog above it, this one has no renderer-side
            // substitute: a `webkitdirectory` input cannot report where the
            // directory it returned actually lives.
            directory_picker::pick_directory_via_dialog,
            check_core_update,
            apply_core_update,
            check_app_update,
            apply_app_update,
            download_app_update,
            install_app_update,
            restart_core_process,
            recover_port_conflict,
            force_quit_port_owner,
            start_core_process,
            local_data_reset::reset_local_data,
            app_quit,
            restart_app,
            get_active_user_id,
            register_dictation_hotkey,
            unregister_dictation_hotkey,
            register_ptt_hotkey,
            unregister_ptt_hotkey,
            ptt_overlay::show_ptt_overlay,
            native_notifications::notification_permission_state,
            native_notifications::notification_permission_request,
            activate_main_window,
            set_titlebar_for_sidebar,
            native_notifications::show_native_notification,
            mascot_window_show,
            mascot_window_hide,
            notch_window_show,
            notch_window_hide,
            file_logging::reveal_logs_folder,
            file_logging::logs_folder_path,
            workspace_paths::open_workspace_path,
            workspace_paths::reveal_workspace_path,
            workspace_paths::preview_workspace_text,
            workspace_paths::resolve_workspace_absolute_path,
            mcp_commands::mcp_resolve_binary_path,
            mcp_commands::mcp_open_client_config,
            loopback_oauth::start_loopback_oauth_listener,
            loopback_oauth::stop_loopback_oauth_listener,
            claude_code::claude_code_login_launch
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |app_handle, event| match event {
            RunEvent::Ready => {
                log::info!("[app] RunEvent::Ready — GTK initialized, setting up tray");
                if let Err(err) = setup_tray(app_handle) {
                    log::warn!(
                    "[tray] failed to setup tray icon (non-fatal in headless environment): {err}"
                );
                }
            }
            // Intercept the main window's close request on macOS so the user
            // can re-open the app from the tray icon. Letting the OS destroy
            // the webview makes `get_webview_window("main")` return None on
            // the next tray-click and surfaces as `[tray] failed to show main
            // window from tray click: main window not found`
            // (OPENHUMAN-TAURI-2X — 21 events, Windows only).
            //
            // macOS: hide the whole application on close instead of
            // destroying the window. `AppHandle::hide()` calls
            // `[NSApp hide:]` via `set_application_visibility(false)`,
            // the standard macOS mechanism that reliably hides all
            // windows. The vendored CEF runtime's per-window
            // `WebviewWindow::hide()` sends `WindowMessage::Hide` →
            // `cef::Window::hide()`, which does NOT propagate to the
            // visible NSWindow frame and leaves the window on screen
            // (issue #2049). Dock-click fires `RunEvent::Reopen` which
            // calls `show_main_window()` to restore.
            //
            // Windows is handled in the separate arm below — see #1607.
            //
            // Linux is left out: `setup_tray` early-returns on Linux
            // because tray creation panics inside GTK during packaged
            // runs, so hide-on-close would strand the user with no way
            // back.
            #[cfg(target_os = "macos")]
            RunEvent::WindowEvent {
                label,
                event: WindowEvent::CloseRequested { api, .. },
                ..
            } if label == "main" => {
                log::info!(
                    "[window] close requested on main window — hiding app"
                );
                api.prevent_close();
                if let Err(err) = app_handle.hide() {
                    log::warn!(
                        "[window] app_handle.hide() failed on close request: {err}"
                    );
                }
            }
            // Windows: full hide-to-tray.
            //
            // PR #1548 routed Windows X click into the same prevent_close +
            // `window.hide()` branch as macOS, but on Windows the vendored
            // CEF runtime's WindowMessage::Hide / Minimize / Restore
            // (`tauri-runtime-cef/src/cef_impl.rs`) only operate on a
            // `cef::Window` internal handle that does not correspond to the
            // visible `Chrome_WidgetWin_1` top-level frame — `ShowWindow`
            // calls against it are silent no-ops. We bypass the runtime
            // entirely and walk the OS window list to issue SW_HIDE / SW_SHOW
            // directly on the matching top-level frame (issue #1607).
            #[cfg(target_os = "windows")]
            RunEvent::WindowEvent {
                label,
                event: WindowEvent::CloseRequested { api, .. },
                ..
            } if label == "main" => {
                log::info!(
                    "[window] close requested on main window — hiding to tray"
                );
                api.prevent_close();
                // Persist geometry now, while the window handle is still
                // reachable. On Windows the hide below is a raw SW_HIDE on the
                // OS frame, after which `get_webview_window("main")` returns
                // `None` until the window is shown again (#1607). If the user
                // then picks tray "Quit" while hidden, the ExitRequested save
                // finds no window and nothing is persisted, so the next launch
                // falls back to the default geometry (#4810). Saving here
                // captures the last on-screen size/position before it becomes
                // unreachable; ExitRequested still saves for the shown-window
                // quit paths (`save_main` is best-effort and idempotent).
                if let Some(window) = app_handle.get_webview_window("main") {
                    window_state::save_main(&window);
                }
                // Hide the OS top-level Chrome_WidgetWin_1 frame via
                // EnumWindows + SW_HIDE — full hide-to-tray as PR #1548
                // intended. `window.hide()` and `window.minimize()` through
                // the vendored CEF runtime are no-ops on Windows because
                // `WebviewWindow::hwnd()` returns a cef::Window proxy handle
                // rather than the visible top-level frame; we walk the OS
                // window list directly instead (#1607). SW_HIDE on the host
                // frame cascades to all child HWNDs (including the CEF
                // browser surface), so no separate `webview.hide()` is
                // needed and `show_main_window` only has to issue SW_SHOW.
                set_main_window_hidden(true);
            }
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { .. } => {
                log::info!("[window] reopen event — showing main window");
                if let Err(err) = show_main_window(app_handle) {
                    log::error!("[macos] failed to show main window on reopen: {err}");
                }
            }
            RunEvent::ExitRequested { .. } => {
                // Persist the main window's geometry on every clean quit
                // (Cmd+Q, tray "Quit", dock quit, or the frontend
                // `app_quit` command) so the next launch restores the
                // user's size + position. Previously `save_main` ran only
                // on the identity-flip `restart_app` path (#900): a normal
                // quit never saved, so the window always reopened at the
                // default small centered size (#4810). `save_main` is
                // best-effort and idempotent — safe to also run here even
                // though `restart_app` saves before it triggers exit.
                if let Some(window) = app_handle.get_webview_window("main") {
                    window_state::save_main(&window);
                }
                // Run cleanup during ExitRequested so the embedded core and
                // long-lived scanner tasks stop before the runtime exits.
                // Teardown stays non-blocking on the main thread.
                perform_early_teardown_sync_once(app_handle, "exit_requested");
            }
            RunEvent::Exit => {
                log::info!("[app] RunEvent::Exit — cef::shutdown follows");
            }
            _ => {}
        });

    // Belt-and-suspenders sweep: after Tauri's event loop returns the
    // vendored runtime has already called `cef::shutdown()`. In normal
    // operation every CEF helper (GPU / Network / Utility / Renderer) is
    // gone by now. If anything is still alive — e.g. a renderer that was
    // mid-spawn when the user quit — it would otherwise be re-parented to
    // launchd on macOS / init on Linux and survive the GUI exit. Sweep its
    // children before this process actually exits.
    //
    // We don't `wait()` on them: the kernel will reap them as our exit
    // unwinds. Give stubborn helpers a short grace period, then force-kill
    // anything still parented to us so the GUI exit leaves no background
    // processes behind.
    process_kill::sweep_orphan_children();
}

pub fn run_core_from_args(args: &[String]) -> Result<(), String> {
    // Core lives in-process: dispatch directly through the linked `openhuman_core`
    // library instead of shelling out to a separate binary. The Tauri main()
    // routes `OpenHuman core <args>` here so users can still drive the core CLI
    // from the bundled app.
    openhuman_core::run_core_from_args(args).map_err(|e| format!("{e:#}"))
}

// ---------------------------------------------------------------------------
// Sentry release / environment resolution (Tauri shell — desktop only)
// ---------------------------------------------------------------------------

/// Canonical release tag: `openhuman@<version>[+<short_sha>]`.
///
/// Mirrors `build_release_tag` in the core sidecar's `src/main.rs` and the
/// `SENTRY_RELEASE` value computed in `app/vite.config.ts` so events from
/// every surface (React frontend, core sidecar, Tauri shell) group under the
/// same release in Sentry and benefit from the same source-map / debug-info
/// upload.
/// Return `true` when the Sentry event is a "Failed to request
/// http://localhost:…" message originating from the vendored
/// `tauri-runtime-cef` dev-server proxy.
///
/// The proxy logs this message via `log::error!` (see
/// `app/src-tauri/vendor/tauri-cef/crates/tauri/src/protocol/tauri.rs`)
/// every time the CEF webview asks for an asset on the Vite dev URL
/// (`http://localhost:1420` per `tauri.conf.json`). In packaged
/// staging/production builds Vite isn't running, so the request fails —
/// but the failure is benign and shouldn't be reported.
///
/// The match is conservative: it checks the exact `Failed to request ` +
/// `http://localhost` / `http://127.0.0.1` prefix that only the dev-proxy
/// emits. Production HTTP errors from elsewhere in the shell or core use
/// different message shapes and won't be filtered.
fn event_is_localhost_dev_fetch_noise(event: &sentry::protocol::Event<'_>) -> bool {
    // sentry-tracing 0.47 (with default `attach_stacktrace=false`) stores the
    // log message in `event.message`. Check there first; fall back to the
    // last exception's `value` for the (currently unused) stacktrace-enabled
    // path so the filter stays correct if attach_stacktrace ever flips.
    let direct = event.message.as_deref();
    let from_exception = event.exception.last().and_then(|e| e.value.as_deref());
    [direct, from_exception]
        .into_iter()
        .flatten()
        .any(message_is_localhost_dev_fetch_noise)
}

/// Pure prefix check, separated from `event_is_localhost_dev_fetch_noise`
/// so the matching rule can be unit-tested without constructing a full
/// Sentry `Event`.
fn message_is_localhost_dev_fetch_noise(message: &str) -> bool {
    // The tauri-cef dev proxy formats the message as:
    //   `Failed to request {url}: {err}`
    // so anchoring on `Failed to request http://localhost` / `127.0.0.1` is
    // sufficient and avoids matching unrelated "Failed to request …" errors
    // elsewhere in the codebase that target real hosts.
    //
    // Note: no `[::1]` (IPv6 loopback) entry — the vendored tauri-cef dev
    // proxy resolves `localhost` to IPv4 via reqwest's default resolver, so
    // dev-server fetches always surface as `http://localhost:` or
    // `http://127.0.0.1:`. Add an `[::1]` prefix if that ever changes
    // (per graycyrus note on PR #1545).
    const PREFIXES: &[&str] = &[
        "Failed to request http://localhost:",
        "Failed to request http://127.0.0.1:",
    ];
    PREFIXES.iter().any(|p| message.starts_with(p))
}

fn build_sentry_release_tag() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let sha = option_env!("OPENHUMAN_BUILD_SHA").unwrap_or("").trim();
    let sha_short: String = sha.chars().take(12).collect();
    if sha_short.is_empty() {
        format!("openhuman@{version}")
    } else {
        format!("openhuman@{version}+{sha_short}")
    }
}

/// Resolve the Sentry environment tag from `OPENHUMAN_APP_ENV` (runtime) or
/// `VITE_OPENHUMAN_APP_ENV` (compile-time fallback). Defaults to
/// `production` so unmarked release builds don't pollute the dev/staging
/// streams.
fn resolve_sentry_environment() -> String {
    if let Ok(value) = std::env::var("OPENHUMAN_APP_ENV") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Some(value) = option_env!("VITE_OPENHUMAN_APP_ENV") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    "production".to_string()
}

/// Returns the macOS product version string (e.g. `"14.5"`) by reading
/// `sw_vers -productVersion`. Returns `None` on non-macOS targets or when
/// the command is unavailable. Used to tag Sentry events and startup logs
/// with OS version so Intel-specific crashes (issue #1012) can be filtered
/// by macOS release.
#[cfg(target_os = "macos")]
fn macos_os_version() -> Option<String> {
    std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
