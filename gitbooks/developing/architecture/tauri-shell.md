---
description: The desktop host (`crates/openhuman-app/`) - Tauri v2 + WebView, IPC, embedded core lifecycle, core bridge.
icon: desktop
---

# Tauri shell (`crates/openhuman-app/`)

The desktop host for OpenHuman: Tauri v2 + WebView, IPC commands, window management, and bridging to the embedded `openhuman-core` Rust runtime (core JSON-RPC). It does **not** duplicate the full domain stack; that lives in `crates/openhuman-core` (library `openhuman_core`; the `openhuman-core` binary is `crates/openhuman-cli/src/main.rs`).

## Responsibilities

1. **Web UI**. Load the Vite build from `app/dist` (or dev server on port 1420).
2. **IPC**. Expose an explicit set of Tauri commands (see [Commands](#tauri-ipc-commands-app-src-tauri)).
3. **Core lifecycle**. Run the core JSON-RPC server as an in-process tokio task (`core_process.rs`) and hand the renderer its URL/bearer via `core_rpc_url` / `core_rpc_token`.
4. **Window + tray**. Desktop window behavior (main, mascot, notch, overlay windows) and system tray (see `lib.rs`).
5. **Session ownership**. Log the user in and keep the current user fresh (`session/`, backed by `openhuman_tinyhumans::session`): exchange the login token, validate the JWT against `GET /auth/me`, cache `/auth/me`, and hand the resulting credential to the core with `auth.set_credential`. The core never talks to the backend's auth endpoints itself.

## Core process model

`app/package.json` `core:stage` is intentionally a no-op kept for script compatibility. The desktop app links the core in-process, so local builds no longer need a staged `openhuman-core-*` sidecar under `crates/openhuman-app/binaries/`.

## Stuck process recovery

Normal app quit runs teardown from `RunEvent::ExitRequested`: long-lived scanner tasks stop, the embedded core's cancellation token is triggered, and the final process sweep sends `SIGTERM` to direct children before escalating holdouts with `SIGKILL` after a short grace period. Sweep summaries are logged as `[app] sweep: term=N kill=M total=K`; any nonzero `kill` count is a warning and means a child ignored graceful shutdown.

On macOS, hard exits (Force Quit, `SIGKILL`, renderer crash) can skip normal teardown. The next launch runs startup recovery before the app runtime starts: it lists OpenHuman processes whose executable path belongs to the launching `.app/Contents`, skips the current process, sends `SIGTERM`, waits briefly, then `SIGKILL`s stragglers that still match the same pid+command. Logs use the `[startup-recovery]` prefix.

Startup recovery skips when `OPENHUMAN_CORE_REUSE_EXISTING=1` is set so manual CLI-core reuse still works. The Tauri command `process_diagnostics_list_owned` returns the currently owned process list; the macOS implementation is bundle-scoped, Linux/Windows currently return empty.

## Tauri shell architecture (`crates/openhuman-app/`)

### Overview

The **`crates/openhuman-app`** crate (Cargo package **`openhuman-app`**, lib **`openhuman`**, binary **`OpenHuman`**) is a **desktop-only** host. It embeds the React UI, registers plugins (deep link, opener, OS, notifications, autostart, updater), manages the main window and tray, and runs the core JSON-RPC server **in-process**.

Non-desktop targets fail at compile time (`compile_error!` in `lib.rs`).

### Directory layout (actual)

`crates/openhuman-app/src/` is a flat set of modules (no `commands/` or `utils/` subtree). Key modules:

```
crates/openhuman-app/src/
├── lib.rs                  # `run()`, tray/menu, plugins, `generate_handler!`, most window/update/lifecycle commands
├── main.rs                 # Binary entry
├── core_process.rs         # CoreProcessHandle: embedded core server task, RPC token, port conflict handling
├── core_rpc.rs             # Auth helpers + `relay_http_rpc` host-side HTTP relay
├── gateway/                # Where the frontend's RPC goes: the core in this process, a
│                           # core at a URL, or one this app provisions in a container /
│                           # over SSH / both (tinybox). types · store · ops · registry ·
│                           # commands
├── imessage_scanner/       # macOS-only: reads ~/Library/Messages/chat.db directly (never used CDP)
├── mascot_native_window.rs / notch_window.rs / window_state.rs
├── dictation_hotkeys.rs / ptt_hotkeys.rs / ptt_overlay.rs
├── native_notifications/
├── artifact_commands.rs    # Artifact export (copy into Downloads)
├── directory_picker.rs     # Native folder picker for the folder memory source (#5831)
├── external_navigation.rs  # Keeps the main webview from navigating off-app; hands remote links to the OS browser
├── workspace_paths.rs      # Safe workspace-relative file open/reveal/preview
├── app_update.rs           # Updater support (commands live in lib.rs)
├── loopback_oauth.rs       # Localhost OAuth redirect listener
├── session/                # Session owner: auth_* commands over openhuman_tinyhumans::session
├── claude_code.rs          # Claude Code login launch
├── mcp_commands.rs         # MCP client helpers
├── file_logging.rs         # Log file sink + logs-folder commands
├── process_recovery.rs / process_kill.rs / local_data_reset.rs  # process_recovery has per-platform submodules under process_recovery/
├── deep_link_ipc.rs / deep_link_ipc_windows.rs / deep_link_registration_check.rs
└── stderr_panic_hook.rs / reset_reboot_schedule.rs
```

This listing was rewritten against the real tree after #5478 / #5456. Gone with
the move off Chromium: `cdp/`, the `cef_*` preflight modules, `webview_accounts/`,
every `*_scanner/` but `imessage_scanner/`, the `meet_*` call window,
`fake_camera/` and `companion_commands.rs`. `webview_apis/` went later: it was
the WS bridge those scanners called, and once they were gone its router
dispatched nothing while still binding a loopback listener at boot.

There is **no** `src-tauri/src/services/session_service.rs` in this tree; session semantics are handled in the web layer + backend + core as applicable.

### Data flow: UI → core

```
React (fetch)
    → POST http://127.0.0.1:<port>/rpc   (URL from `core_rpc_url`,
                                          bearer from `core_rpc_token`)
        → embedded openhuman core server (tokio task in this process)
```

The renderer talks to the local core **directly over HTTP**: `app/src/services/coreRpcClient.ts` invokes `core_rpc_url` / `core_rpc_token` once, then issues plain `fetch()` calls. The `relay_http_rpc` Tauri command is a host-side fallback used only when the RPC URL is **not** a trustworthy origin for the secure `tauri://localhost` webview (e.g. a self-hosted runtime on a LAN IP, blocked as mixed content, #3865). In that case the Rust host delegates to `openhuman_rpc::post_json_rpc` from the shared `crates/openhuman-rpc` crate (feature `http-client`): 30 s timeout, redirects disabled when a bearer is present, status + body mirrored back verbatim as `HttpRpcResponse`. The shell adds only the gateway transport guard (`validate_remote_transport`, feature `gateways`) before delegating.

`CoreProcessHandle` in `core_process.rs` owns the embedded server task (started via `openhuman_rpc::server::run_server_embedded_with_ready` with a per-launch random bearer token) and handles stale-listener/port-conflict recovery.

### Window and tray behavior

- The shell creates a tray icon at startup (`RunEvent::Ready`) and wires actions to open the main window or quit. Tray setup is skipped on Linux packaged runs (GTK panic).
- Hide-to-tray is implemented in the `RunEvent::WindowEvent { CloseRequested }` handlers in `lib.rs`, not as IPC commands: macOS hides the whole app (`AppHandle::hide()`, #2049), Windows hides the top-level `Chrome_WidgetWin_1` frame via `EnumWindows` + `SW_HIDE` (#1607).
- On macOS `RunEvent::Reopen` (Dock click) restores and focuses the main window.

### Bundled resources

`tauri.conf.json` bundles **`../../crates/openhuman-core/src/agent/prompts`** so the core prompt markdown ships with the app.

### Related

- IPC surface: see the [Commands](#tauri-ipc-commands-app-src-tauri) section below
- HTTP bridge: see the [Core bridge & helpers](#core-bridge-helpers-app-src-tauri) section below
- Rust domains and runtime: `crates/openhuman-core/src/`, `crates/openhuman-core/src/core/`
- Shared RPC contracts + HTTP client: `crates/openhuman-rpc/` (also used by `crates/openhuman-tui` for envelope decoding)

## Tauri IPC commands (`crates/openhuman-app`)

All commands are registered in **`crates/openhuman-app/src/lib.rs`** inside `tauri::generate_handler![...]`: that list is the authoritative reference. The major families:

### Core RPC & diagnostics

| Command                          | Purpose                                                                                                                                         |
| -------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `core_rpc_url`                   | Return the **active gateway's** JSON-RPC URL: the embedded core's `http://127.0.0.1:<port>/rpc` unless another gateway is active               |
| `core_rpc_token`                 | Return the active gateway's bearer. Paired with `core_rpc_url`: a token minted for the embedded core is meaningless to a core in a container    |
| `core_rpc_endpoint`              | Return the URL and bearer together from one resolved snapshot, so a gateway switch between two separate calls cannot pair one gateway's URL with another's token |
| `relay_http_rpc`                 | Host-side JSON-RPC POST (`{ url, token?, body }` → `{ status, body }`) for self-hosted runtimes the webview cannot fetch (mixed content, #3865) |
| `overlay_parent_rpc_url`         | RPC URL inherited from a parent process (overlay windows), from `OPENHUMAN_CORE_RPC_URL`                                                        |
| `process_diagnostics_list_owned` | List OpenHuman processes owned by this app bundle (macOS; empty elsewhere)                                                                      |

Use **`app/src/services/coreRpcClient.ts`** (`callCoreRpc`) from the frontend.

### Gateways: running the core somewhere else

A **gateway** is one way of reaching an OpenHuman core. Four exist: the core inside this
process, a core somebody else is running at a URL, and two this app provisions itself: in
a Docker container, or on a machine reached over SSH. The last two are the same code:
[tinybox](https://github.com/tinyhumansai/tinybox) models *reach* (`local` / `ssh`) and
*confinement* (`passthrough` / `docker`) as independent axes, so "a container on the build
server" is those two choices made separately rather than a third case with code of its own.

**The seam is one line.** A gateway resolves to a URL and a bearer, and `core_rpc_url` /
`core_rpc_token` answer from the active one. Every RPC call site in the renderer therefore
follows along unchanged: there is no per-gateway transport in the frontend, and
`services/transport/` (the iOS `ConnectionProfile` path) is not involved.

Provisioning is four tinybox calls: `create` a box publishing the core's port, `spawn` the
core in it detached with a freshly minted bearer, `forward` that published port back to
this machine, then poll the core's unauthenticated `/health`. The third step is the one
that is easy to omit and impossible to notice missing: publishing puts the port on the
*box's* host, which for an SSH placement is the far machine.

| Command            | Purpose                                                                    |
| ------------------ | -------------------------------------------------------------------------- |
| `gateway_list`     | Every configured gateway, the built-in desktop one first                    |
| `gateway_save`     | Add or replace a gateway. Does not activate it                              |
| `gateway_delete`   | Forget a gateway. The running session is unaffected                         |
| `gateway_activate` | Provision if needed, then make it the one RPC goes to                       |
| `gateway_active`   | Which gateway is active                                                     |
| `gateway_status`   | `inactive` / `activating{step}` / `connected{endpoint}` / `failed{reason}`  |

Records live shell-side in `gateways.json`, **not** renderer `localStorage`: an SSH
identity path and a remote bearer are materially more sensitive than a window position, and
the renderer's own notes on the cloud token (audit U3, `utils/configPersistence.ts`) already
say a renderer XSS can read anything kept there. The frontend holds a gateway *id*.

Shell-internal callers (`imessage_scanner`, `local_data_reset`) deliberately
keep talking to the embedded core: they are about *this* machine's iMessage database, *this*
install's data, and *this* machine's audio, so routing them to a remote gateway would be
wrong rather than incomplete.

Gated by the shell-local `gateways` Cargo feature (default on). That gate is unrelated to
the feature-forwarding rules in `AGENTS.md`, which govern which `openhuman_core` gates the
shell forwards; nothing here belongs in `scripts/ci/product-features.txt`.

Frontend: **`app/src/services/gatewayService.ts`**, surfaced in Settings → Core connection
(`components/settings/panels/core/GatewaySection.tsx`).

### Core & app lifecycle

| Command                                           | Purpose                                       |
| ------------------------------------------------- | --------------------------------------------- |
| `start_core_process` / `restart_core_process`     | Start / restart the embedded core server task |
| `recover_port_conflict` / `force_quit_port_owner` | Resolve a foreign listener on the core port   |
| `reset_local_data`                                | Wipe local app data (`local_data_reset.rs`)   |
| `app_quit` / `restart_app`                        | Quit or relaunch the app                      |
| `get_active_user_id`                              | Read the active user id                       |

### Session (`session/`)

The shell is the session owner on the desktop: it talks to the TinyHumans backend's auth endpoints so the core never has to. `openhuman_tinyhumans::session` does the work; the shell adds the link to the core (`HttpCoreLink`, the same `(url, token)` the renderer uses, so a gateway switch is followed) and these commands. Errors carry a stable `PREFIX:` (`REJECTED`, `EXPIRED`, `TRANSIENT`, `CONSUME_FAILED`, `USER_ID_UNAVAILABLE`, `CORE`) the renderer classifies on.

| Command                 | Purpose                                                                                                                        |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `auth_login_with_token` | Exchange a one-time login token (`POST /auth/login-token/consume`), validate the JWT (`GET /auth/me`), install it in the core |
| `auth_store_session`    | Install a JWT the renderer already holds (validated first) or the offline local token (stored as-is with its user)            |
| `auth_logout`           | Clear the session credential in the core                                                                                       |
| `auth_state`            | The core's credential state plus the cached current user                                                                       |
| `auth_current_user`     | The current user from the `/auth/me` cache (5 s TTL, stale-while-revalidate, backoff); `force` bypasses the cache             |

Events: `auth://changed` (credential or current user changed; payload `SessionState`) and `auth://expired` (the backend rejected the stored credential and it has been cleared; payload `{ source }`). `app/src/services/session/shellSessionEvents.ts` bridges them into the window events `CoreStateProvider` already handles.

A JWT accepted while the backend is unreachable (live `exp`, subject claim) is installed with a `pendingBackendValidation` placeholder user and revalidated in the background; a rejected JWT is never installed. In cloud mode the renderer targets a remote core the shell does not know about, so `app/src/services/session/sessionOwner.ts` runs a thin browser equivalent there (and in the browser build).

### Updates

`check_core_update` / `apply_core_update` (embedded core) and `check_app_update` / `download_app_update` / `install_app_update` / `apply_app_update` (desktop app, via the updater plugin).

### Hotkeys (dictation, PTT)

| Command                                                                            | Purpose                                                                       |
| ---------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `register_dictation_hotkey` / `unregister_dictation_hotkey`                        | Global dictation shortcuts (`dictation_hotkeys.rs`)                           |
| `register_ptt_hotkey` / `unregister_ptt_hotkey` / `show_ptt_overlay`               | Push-to-talk: see the [PTT section](#push-to-talk-ptt-hotkey--overlay) below |

### Notifications

`notification_permission_state` / `notification_permission_request` / `show_native_notification` provide the OS-level notification surface. They are implemented in `native_notifications`.

### Window management

| Command                                            | Purpose                                        |
| -------------------------------------------------- | ---------------------------------------------- |
| `activate_main_window`                             | Show + focus the main window                   |
| `set_titlebar_for_sidebar`                         | macOS only: switch the main window's title bar style when the sidebar collapses, so the webview keeps spanning the full window |
| `mascot_window_show` / `mascot_window_hide`        | Toggle the mascot native window                |
| `notch_window_show` / `notch_window_hide`          | Toggle the notch window                        |

Hide-to-tray / reopen behavior is **not** an IPC command: it lives in the `RunEvent` handlers in `lib.rs` (see [Window and tray behavior](#window-and-tray-behavior)).

### Artifacts, logs, MCP, OAuth

| Command                                                          | Purpose                                                                                 |
| ---------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| `download_artifact_to_downloads`                                 | Copy an artifact straight to the Downloads folder (`artifact_commands.rs`); the Save-As dialog this used to sit behind was dropped with the shell's `rfd` dependency |
| `pick_directory_via_dialog`                                      | Native OS folder picker for the folder memory source, returning an absolute path a `webkitdirectory` input cannot report (`directory_picker.rs`, #5831) |
| `reveal_logs_folder` / `logs_folder_path`                        | Open / return the file-logging folder (`file_logging.rs`)                               |
| `mcp_resolve_binary_path` / `mcp_open_client_config`             | MCP client helpers (`mcp_commands.rs`)                                                  |
| `start_loopback_oauth_listener` / `stop_loopback_oauth_listener` | Localhost OAuth redirect listener (`loopback_oauth.rs`)                                 |
| `claude_code_login_launch`                                       | Launch the Claude Code login flow (`claude_code.rs`)                                    |

### Workspace file links

From **`workspace_paths.rs`** (closes `#1402`). These commands accept workspace-relative paths only. The shell resolves each path against the active OpenHuman workspace, canonicalizes the target, and rejects traversal, absolute paths, URI-like prefixes, and symlink escapes before opening or reading anything.

| Command                           | Purpose                                                                |
| --------------------------------- | ---------------------------------------------------------------------- |
| `open_workspace_path`             | Open an existing workspace file or directory with the OS default app.  |
| `reveal_workspace_path`           | Reveal an existing workspace file or directory in the OS file manager. |
| `preview_workspace_text`          | Read a capped UTF-8 text preview from an existing workspace file.      |

### Push-to-talk (PTT) hotkey + overlay

Registered in **`lib.rs`** (`ptt_hotkeys.rs` + `ptt_overlay.rs`). These commands manage the global push-to-talk shortcut and the floating overlay window.

| Command                 | Signature                                  | Purpose                                                                                                                                                                                                                                                                                                                        |
| ----------------------- | ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `register_ptt_hotkey`   | `(shortcut: String) -> Result<(), String>` | Register (or re-register) a global hotkey for push-to-talk. Emits Tauri events `ptt://start { session_id }` (key pressed) and `ptt://stop { session_id }` (key released). Returns an error string if the shortcut conflicts with dictation or if the OS rejects it (e.g. Wayland, Accessibility permission required on macOS). |
| `unregister_ptt_hotkey` | `() -> Result<(), String>`                 | Unregister the current PTT hotkey and tear down the overlay window.                                                                                                                                                                                                                                                            |
| `show_ptt_overlay`      | `(active: bool, session_id: u64) -> ()`    | Show (`active: true`) or hide (`active: false`) the floating PTT overlay window. The window is focus-stealing-free (`focus: false`). Called by `PttHotkeyManager.tsx` via `app/src/utils/tauriCommands/ptt.ts`.                                                                                                                |

**Event flow:** `register_ptt_hotkey` wires the OS hotkey to fire `ptt://start` / `ptt://stop` Tauri events that `PttHotkeyManager.tsx` subscribes to via `@tauri-apps/api/event`. The manager forwards them into the `pttService` state machine which drives the audio capture → transcribe → chat-send pipeline.

**Conflict detection:** `register_ptt_hotkey` checks for overlap with the active dictation shortcuts before registering. If a conflict is detected it returns `"ConflictsWithDictation(<shortcut>)"` without registering anything, and the settings panel surfaces this as `pttSettings.errorConflictsWithDictation`.

### Synthetic input main-thread executor (native registry, not `invoke`)

Registered in **`lib.rs`** at startup under the event-bus native-request method
`computer.input_on_main_thread` (`INPUT_ON_MAIN_THREAD_METHOD`, defined in
`openhuman_core::tools::computer::main_thread`). This is **not** a
`@tauri-apps/api` `invoke` command. It is an in-process native request the
**core** dispatches to the **shell** so synthetic input runs on the real app
main thread.

Why: enigo's macOS keyboard-layout lookup (`TSMGetInputSourceProperty`) traps
(`_dispatch_assert_queue_fail` / `EXC_BREAKPOINT`) and crashes the desktop host when
called off the main thread. The `mouse` / `keyboard` tools therefore never call
enigo on their tokio worker; they build a closure and dispatch it here, where
the shell runs it via `AppHandle::run_on_main_thread`.

| Field        | Shape                                                                                               |
| ------------ | --------------------------------------------------------------------------------------------------- |
| Method       | `computer.input_on_main_thread`                                                                     |
| Request      | `MainThreadInputOp { run: Box<dyn FnOnce() -> Result<String, String> + Send> }` (passed by value)   |
| Response     | `Result<String, String>`: `Ok(message)` on success, `Err(reason)` on failure                        |
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

_See `crates/openhuman-app/src/lib.rs` (`generate_handler!`) for the authoritative list._

## Core bridge & helpers (`crates/openhuman-app`)

The Tauri crate **does not** embed a duplicate Socket.io server or Telegram client; it focuses on **in-process core lifecycle** and the thin HTTP/auth glue around the core's JSON-RPC surface.

### `CoreProcessHandle` (`core_process.rs`)

- Runs the core's HTTP/JSON-RPC server as a **tokio task inside the Tauri host** via `openhuman_rpc::server::run_server_embedded_with_ready`: no sidecar binary.
- Generates a per-launch 256-bit hex bearer token (`generate_rpc_token`) and hands it to the embedded server; the renderer reads it via the `core_rpc_token` command.
- Stale-listener policy (#1130): if the core port is already occupied, probes whether the listener is an old OpenHuman core (terminate + respawn) or something foreign (surface the conflict). `OPENHUMAN_CORE_REUSE_EXISTING=1` opts back into attach-to-existing for debugging.
- Managed as Tauri state in `lib.rs` (`app.manage(core_handle)`).

### `core_rpc` (`core_rpc.rs`)

- Shared auth helpers for host-side calls to the local core (URL from `OPENHUMAN_CORE_RPC_URL` or the default port; bearer from `core_process::current_rpc_token`).
- **`relay_http_rpc`** Tauri command: a thin wrapper over `openhuman_rpc::post_json_rpc` (`crates/openhuman-rpc`, feature `http-client`) for self-hosted runtimes on non-trustworthy origins; `bearer_header`, `redact_url_for_log` and `HttpRpcResponse` are re-exported from that crate (see [Core RPC & diagnostics](#core-rpc--diagnostics)).
