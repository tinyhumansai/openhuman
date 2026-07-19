---
description: The desktop host (`app/src-tauri/`) - Tauri v2 + WebView, IPC, embedded core lifecycle, core bridge.
icon: desktop
---

# Tauri shell (`app/src-tauri/`)

The desktop host for OpenHuman: Tauri v2 + WebView, IPC commands, window management, and bridging to the embedded `openhuman-core` Rust runtime (core JSON-RPC). It does **not** duplicate the full domain stack; that lives in the repo-root Rust crate (`openhuman_core`, `src/main.rs`).

## Responsibilities

1. **Web UI**. Load the Vite build from `app/dist` (or dev server on port 1420).
2. **IPC**. Expose an explicit set of Tauri commands (see [Commands](#tauri-ipc-commands-app-src-tauri)).
3. **Core lifecycle**. Run the core JSON-RPC server as an in-process tokio task (`core_process.rs`) and hand the renderer its URL/bearer via `core_rpc_url` / `core_rpc_token`.
4. **Provider webviews**. Host embedded CEF child webviews for channel providers (`webview_accounts/`, per-provider scanners) and their CDP plumbing (`cdp/`).
5. **Window + tray**. Desktop window behavior (main, mascot, notch, overlay windows) and system tray (see `lib.rs`).

## Core process model

`app/package.json` `core:stage` is intentionally a no-op kept for script compatibility. The desktop app links the core in-process, so local builds no longer need a staged `openhuman-core-*` sidecar under `app/src-tauri/binaries/`.

## Stuck process recovery

Normal app quit runs teardown from `RunEvent::ExitRequested`: child webviews are closed before CEF shutdown, the embedded core's cancellation token is triggered, and the final process sweep sends `SIGTERM` to direct children before escalating holdouts with `SIGKILL` after a short grace period. Sweep summaries are logged as `[app] sweep: term=N kill=M total=K`; any nonzero `kill` count is a warning and means a child ignored graceful shutdown.

On macOS, hard exits (Force Quit, `SIGKILL`, renderer crash) can skip normal teardown. The next launch runs startup recovery before CEF cache preflight: it lists OpenHuman processes whose executable path belongs to the launching `.app/Contents`, skips the current process, sends `SIGTERM`, waits briefly, then `SIGKILL`s stragglers that still match the same pid+command. Logs use the `[startup-recovery]` prefix.

Startup recovery skips when `OPENHUMAN_CORE_REUSE_EXISTING=1` is set (so manual CLI-core reuse still works) and when the CEF `SingletonLock` is held by a live process (so the normal second-instance path can fail without killing the already-running app). The Tauri command `process_diagnostics_list_owned` returns the currently owned process list; the macOS implementation is bundle-scoped, Linux/Windows currently return empty.

## Tauri shell architecture (`app/src-tauri/`)

### Overview

The **`app/src-tauri`** crate (Rust package **`OpenHuman`**, binary **`OpenHuman`**) is a **desktop-only** host. It embeds the React UI, registers plugins (deep link, opener, OS, notifications, autostart, updater), manages the main window and tray, and runs the core JSON-RPC server **in-process**.

Non-desktop targets fail at compile time (`compile_error!` in `lib.rs`).

### Directory layout (actual)

`app/src-tauri/src/` is a flat set of modules (no `commands/` or `utils/` subtree). Key modules:

```
app/src-tauri/src/
├── lib.rs                  # `run()`, tray/menu, plugins, `generate_handler!`, most window/update/lifecycle commands
├── main.rs                 # Binary entry
├── core_process.rs         # CoreProcessHandle — embedded core server task, RPC token, port conflict handling
├── core_rpc.rs             # Auth helpers + `relay_http_rpc` host-side HTTP relay
├── cdp/                    # Chrome DevTools Protocol plumbing for child webviews
├── cef_preflight.rs / cef_profile.rs / cef_singleton_wait.rs / cef_stale_reap.rs   # CEF cache/profile management
├── webview_accounts/       # Embedded provider account webviews (open/close/bounds/notifications)
├── webview_apis/           # WS bridge for webview-side APIs
├── discord_scanner/ … whatsapp_scanner/ …   # Per-provider scanners (slack, telegram, wechat,
│                                            # gmessages, imessage, meet, …) driving CDP
├── meet_audio/ meet_call/ meet_video/       # Google Meet call window + media capture
├── fake_camera/            # Virtual camera support
├── screen_capture/         # Screen share picker sessions (getDisplayMedia shim backend)
├── mascot_native_window.rs / notch_window.rs / window_state.rs
├── dictation_hotkeys.rs / ptt_hotkeys.rs / ptt_overlay.rs / companion_commands.rs
├── native_notifications/ notification_settings/
├── artifact_commands.rs    # Artifact export (save dialog / Downloads)
├── workspace_paths.rs      # Safe workspace-relative file open/reveal/preview
├── app_update.rs           # Updater support (commands live in lib.rs)
├── loopback_oauth.rs       # Localhost OAuth redirect listener
├── claude_code.rs          # Claude Code login launch
├── mcp_commands.rs         # MCP client helpers
├── file_logging.rs         # Log file sink + logs-folder commands
├── process_recovery.rs / process_kill.rs / local_data_reset.rs
├── deep_link_ipc.rs / deep_link_ipc_windows.rs / deep_link_registration_check.rs
└── stderr_panic_hook.rs / reset_reboot_schedule.rs
```

There is **no** `src-tauri/src/services/session_service.rs` in this tree; session semantics are handled in the web layer + backend + core as applicable.

### Data flow: UI → core

```
React (fetch)
    → POST http://127.0.0.1:<port>/rpc   (URL from `core_rpc_url`,
                                          bearer from `core_rpc_token`)
        → embedded openhuman core server (tokio task in this process)
```

The renderer talks to the local core **directly over HTTP** — `app/src/services/coreRpcClient.ts` invokes `core_rpc_url` / `core_rpc_token` once, then issues plain `fetch()` calls. The `relay_http_rpc` Tauri command is a host-side fallback used only when the RPC URL is **not** a trustworthy origin for the secure `tauri://localhost` webview (e.g. a self-hosted runtime on a LAN IP, blocked as mixed content — #3865): the Rust host performs the POST with `reqwest` and mirrors status + body back verbatim.

`CoreProcessHandle` in `core_process.rs` owns the embedded server task (started via `openhuman_core::core::jsonrpc::run_server_embedded_with_ready` with a per-launch random bearer token) and handles stale-listener/port-conflict recovery.

### Window and tray behavior

- The shell creates a tray icon at startup (`RunEvent::Ready`) and wires actions to open the main window or quit. Tray setup is skipped on Linux packaged runs (GTK panic).
- Hide-to-tray is implemented in the `RunEvent::WindowEvent { CloseRequested }` handlers in `lib.rs`, not as IPC commands: macOS hides the whole app (`AppHandle::hide()`, #2049), Windows hides the top-level `Chrome_WidgetWin_1` frame via `EnumWindows` + `SW_HIDE` (#1607).
- On macOS `RunEvent::Reopen` (Dock click) restores and focuses the main window.

### Bundled resources

`tauri.conf.json` bundles **`../../src/openhuman/agent/prompts`** and **`recipes/**/*`** so prompt markdown and provider recipes ship with the app.

### Related

- IPC surface: see the [Commands](#tauri-ipc-commands-app-src-tauri) section below
- HTTP bridge: see the [Core bridge & helpers](#core-bridge-helpers-app-src-tauri) section below
- Rust domains (implementation): repo root `src/openhuman/`, `src/core/`

## Tauri IPC commands (`app/src-tauri`)

All commands are registered in **`app/src-tauri/src/lib.rs`** inside `tauri::generate_handler![...]` — that list is the authoritative reference. The major families:

### Core RPC & diagnostics

| Command                         | Purpose                                                                                     |
| ------------------------------- | ------------------------------------------------------------------------------------------- |
| `core_rpc_url`                  | Return the local core JSON-RPC URL (`http://127.0.0.1:<port>/rpc`)                         |
| `core_rpc_token`                | Return the per-launch bearer token for the embedded core                                     |
| `relay_http_rpc`                | Host-side JSON-RPC POST (`{ url, token?, body }` → `{ status, body }`) for self-hosted runtimes the webview cannot fetch (mixed content, #3865) |
| `overlay_parent_rpc_url`        | RPC URL inherited from a parent process (overlay windows), from `OPENHUMAN_CORE_RPC_URL`   |
| `process_diagnostics_list_owned`| List OpenHuman processes owned by this app bundle (macOS; empty elsewhere)                  |

Use **`app/src/services/coreRpcClient.ts`** (`callCoreRpc`) from the frontend.

### Core & app lifecycle

| Command | Purpose |
| ------- | ------- |
| `start_core_process` / `restart_core_process` | Start / restart the embedded core server task |
| `recover_port_conflict` / `force_quit_port_owner` | Resolve a foreign listener on the core port |
| `reset_local_data` | Wipe local app data (`local_data_reset.rs`) |
| `app_quit` / `restart_app` | Quit or relaunch the app |
| `get_active_user_id` | Read the active user id |
| `schedule_cef_profile_purge` | Schedule a CEF profile purge for a user |

### Updates

`check_core_update` / `apply_core_update` (embedded core) and `check_app_update` / `download_app_update` / `install_app_update` / `apply_app_update` (desktop app, via the updater plugin).

### Hotkeys (dictation, PTT, companion)

| Command | Purpose |
| ------- | ------- |
| `register_dictation_hotkey` / `unregister_dictation_hotkey` | Global dictation shortcuts (`dictation_hotkeys.rs`) |
| `register_ptt_hotkey` / `unregister_ptt_hotkey` / `show_ptt_overlay` | Push-to-talk — see the [PTT section](#push-to-talk-ptt-hotkey--overlay) below |
| `register_companion_hotkey` / `unregister_companion_hotkey` / `companion_activate` | Companion window hotkey + activation (`companion_commands.rs`) |

### Provider webviews (`webview_accounts::*`)

Lifecycle and layout of embedded CEF account webviews: `webview_account_open` / `_prewarm` / `_close` / `_purge` / `_bounds` / `_reveal` / `_hide` / `_show`, `webview_set_focused_account`, `webview_recipe_event`, plus webview-notification controls (`webview_notification_permission_state` / `_permission_request` / `_set_dnd` / `_mute_account` / `_get_bypass_prefs`).

### Notifications

`notification_settings_get` / `notification_settings_set` (persisted settings) and `native_notifications::notification_permission_state` / `notification_permission_request` / `show_native_notification` (OS-level).

### Window management

| Command | Purpose |
| ------- | ------- |
| `activate_main_window` | Show + focus the main window |
| `mascot_window_show` / `mascot_window_hide` | Toggle the mascot native window |
| `notch_window_show` / `notch_window_hide` | Toggle the notch window |
| `meet_call_open_window` / `meet_call_close_window` | Open/close the Meet call window (`meet_call/`) |

Hide-to-tray / reopen behavior is **not** an IPC command — it lives in the `RunEvent` handlers in `lib.rs` (see [Window and tray behavior](#window-and-tray-behavior)).

### Artifacts, logs, MCP, OAuth

| Command | Purpose |
| ------- | ------- |
| `save_artifact_via_dialog` / `download_artifact_to_downloads` | Export an artifact via Save-As dialog or straight to Downloads (`artifact_commands.rs`) |
| `reveal_logs_folder` / `logs_folder_path` | Open / return the file-logging folder (`file_logging.rs`) |
| `mcp_resolve_binary_path` / `mcp_open_client_config` | MCP client helpers (`mcp_commands.rs`) |
| `start_loopback_oauth_listener` / `stop_loopback_oauth_listener` | Localhost OAuth redirect listener (`loopback_oauth.rs`) |
| `claude_code_login_launch` | Launch the Claude Code login flow (`claude_code.rs`) |

### Screen share picker (CEF / macOS)

From **`screen_capture/mod.rs`**. Backs the in-page `getDisplayMedia` shim in `webview_accounts/runtime.js`. Session-gated: the shim must open a session with a live user gesture before enumeration / thumbnail captures succeed. See issue #713 (picker UX) + #812 (session gating).

| Command                         | Purpose                                                                                                                                                               |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `screen_share_begin_session`    | Open a 30s session from an account webview, after a `navigator.userActivation.isActive` gesture. Returns `{ token, sources }`. Rate-limited to 10/minute per account. |
| `screen_share_thumbnail`        | Capture a single source's thumbnail as base64 PNG. Requires a live token and an `id` that the session was issued for. macOS only; other platforms return an error.    |
| `screen_share_finalize_session` | Close the session. Called by the shim on Share or Cancel; safe to call with an unknown/expired token (no-op).                                                         |

### Workspace file links

From **`workspace_paths.rs`** (closes `#1402`). These commands accept workspace-relative paths only. The shell resolves each path against the active OpenHuman workspace, canonicalizes the target, and rejects traversal, absolute paths, URI-like prefixes, and symlink escapes before opening or reading anything.

| Command                           | Purpose                                                                |
| --------------------------------- | ---------------------------------------------------------------------- |
| `open_workspace_path`             | Open an existing workspace file or directory with the OS default app.  |
| `reveal_workspace_path`           | Reveal an existing workspace file or directory in the OS file manager. |
| `preview_workspace_text`          | Read a capped UTF-8 text preview from an existing workspace file.      |
| `resolve_workspace_absolute_path` | Resolve a workspace-relative path to its validated absolute path.      |

### Push-to-talk (PTT) hotkey + overlay

Registered in **`lib.rs`** (`ptt_hotkeys.rs` + `ptt_overlay.rs`). These commands manage the global push-to-talk shortcut and the floating overlay window.

| Command                | Signature                                      | Purpose                                                                                                                                                            |
| ---------------------- | ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `register_ptt_hotkey`  | `(shortcut: String) -> Result<(), String>`     | Register (or re-register) a global hotkey for push-to-talk. Emits Tauri events `ptt://start { session_id }` (key pressed) and `ptt://stop { session_id }` (key released). Returns an error string if the shortcut conflicts with dictation or if the OS rejects it (e.g. Wayland, Accessibility permission required on macOS). |
| `unregister_ptt_hotkey`| `() -> Result<(), String>`                     | Unregister the current PTT hotkey and tear down the overlay window.                                                                                                |
| `show_ptt_overlay`     | `(active: bool, session_id: u64) -> ()`        | Show (`active: true`) or hide (`active: false`) the floating PTT overlay window. The window is focus-stealing-free (`focus: false`). Called by `PttHotkeyManager.tsx` via `app/src/utils/tauriCommands/ptt.ts`. |

**Event flow:** `register_ptt_hotkey` wires the OS hotkey to fire `ptt://start` / `ptt://stop` Tauri events that `PttHotkeyManager.tsx` subscribes to via `@tauri-apps/api/event`. The manager forwards them into the `pttService` state machine which drives the audio capture → transcribe → chat-send pipeline.

**Conflict detection:** `register_ptt_hotkey` checks for overlap with the active dictation shortcuts before registering. If a conflict is detected it returns `"ConflictsWithDictation(<shortcut>)"` without registering anything, and the settings panel surfaces this as `pttSettings.errorConflictsWithDictation`.

### Synthetic input main-thread executor (native registry, not `invoke`)

Registered in **`lib.rs`** at startup under the event-bus native-request method
`computer.input_on_main_thread` (`INPUT_ON_MAIN_THREAD_METHOD`, defined in
`openhuman_core::openhuman::tools::computer::main_thread`). This is **not** a
`@tauri-apps/api` `invoke` command. It is an in-process native request the
**core** dispatches to the **shell** so synthetic input runs on the real app
main thread.

Why: enigo's macOS keyboard-layout lookup (`TSMGetInputSourceProperty`) traps
(`_dispatch_assert_queue_fail` / `EXC_BREAKPOINT`) and crashes the CEF host when
called off the main thread. The `mouse` / `keyboard` tools therefore never call
enigo on their tokio worker; they build a closure and dispatch it here, where
the shell runs it via `AppHandle::run_on_main_thread`.

| Field        | Shape                                                                                              |
| ------------ | -------------------------------------------------------------------------------------------------- |
| Method       | `computer.input_on_main_thread`                                                                    |
| Request      | `MainThreadInputOp { run: Box<dyn FnOnce() -> Result<String, String> + Send> }` (passed by value)  |
| Response     | `Result<String, String>`: `Ok(message)` on success, `Err(reason)` on failure                      |
| Availability | Desktop only. Headless / CLI builds register no executor; the core call then returns a clean `Err`. |

### Removed / not present

The following **do not** exist in the current `generate_handler!` list: `greet`, `core_rpc_relay` (superseded by direct `fetch` + `relay_http_rpc`), `ai_get_config` / `ai_refresh_config` / `write_ai_config_file`, `show_window` / `hide_window` / `toggle_window` / `minimize_window` / `maximize_window` / `close_window`, the `openhuman_*` daemon/service helpers, `exchange_token`, `get_auth_state`, `socket_connect`, `start_telegram_login`. Authentication and sockets are handled in the **React** app and **core** process, not via these IPC names.

### Example: core RPC

```typescript
import { callCoreRpc } from "../services/coreRpcClient"; // app/src/services/coreRpcClient.ts

// Direct HTTP to the embedded core (URL + bearer resolved via
// `core_rpc_url` / `core_rpc_token` under the hood):
const result = await callCoreRpc({
  method: "your.rpc.method",
  params: { foo: "bar" },
});
```

---

_See `app/src-tauri/src/lib.rs` (`generate_handler!`) for the authoritative list._

## Core bridge & helpers (`app/src-tauri`)

The Tauri crate **does not** embed a duplicate Socket.io server or Telegram client; it focuses on **in-process core lifecycle** and the thin HTTP/auth glue around the core's JSON-RPC surface.

### `CoreProcessHandle` (`core_process.rs`)

- Runs the core's HTTP/JSON-RPC server as a **tokio task inside the Tauri host** via `openhuman_core::core::jsonrpc::run_server_embedded_with_ready` — no sidecar binary.
- Generates a per-launch 256-bit hex bearer token (`generate_rpc_token`) and hands it to the embedded server; the renderer reads it via the `core_rpc_token` command.
- Stale-listener policy (#1130): if the core port is already occupied, probes whether the listener is an old OpenHuman core (terminate + respawn) or something foreign (surface the conflict). `OPENHUMAN_CORE_REUSE_EXISTING=1` opts back into attach-to-existing for debugging.
- Managed as Tauri state in `lib.rs` (`app.manage(core_handle)`).

### `core_rpc` (`core_rpc.rs`)

- Shared auth helpers for host-side calls to the local core (URL from `OPENHUMAN_CORE_RPC_URL` or the default port; bearer from `core_process::current_rpc_token`).
- **`relay_http_rpc`** Tauri command: host-side `reqwest` POST for self-hosted runtimes on non-trustworthy origins (see [Core RPC & diagnostics](#core-rpc--diagnostics)).
