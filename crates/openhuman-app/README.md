# openhuman-app

The desktop host for OpenHuman on Windows, macOS and Linux. It is a thin Tauri
v2 application on Wry (WKWebView, WebView2, WebKitGTK) that links the Rust core
as a library and runs the core's HTTP/JSON-RPC server as a tokio task inside
the GUI process. The React frontend in [`app/`](../../app/) runs in the main webview and
talks to that core over loopback HTTP. Everything else in this crate is
platform glue the webview cannot do on its own: windows and tray, deep links,
native notifications, global hotkeys, auto-update, login, and process
recovery.

Three names refer to this crate. The Cargo package is `openhuman-app`, the
library is `openhuman`, and the executable is `OpenHuman`. Only the package
name follows the [`crates/`](../) layout; the library and binary names are shipped
identities.

For the user-facing tour of windows, tray and data flow, see
[`gitbooks/developing/architecture/tauri-shell.md`](../../gitbooks/developing/architecture/tauri-shell.md).

## How it works

### Process shape

```text
+---------------------------- OpenHuman process -----------------------------+
|                                                                           |
|   main webview (React, app/src)            Tauri commands (lib.rs)        |
|   +---------------------------+  invoke   +----------------------------+  |
|   | coreRpcClient             |---------->| core_rpc_endpoint          |  |
|   |   fetch(url, Bearer token)|<----------|   -> (url, token)          |  |
|   +-------------+-------------+           | session::commands (auth_*) |  |
|                 |                         | gateway::commands          |  |
|                 | HTTP POST /rpc          | hotkeys, windows, update   |  |
|                 v                         +-------------+--------------+  |
|   +---------------------------+                         |                 |
|   | embedded core server      |<------------------------+                 |
|   | openhuman_rpc::host       |   shell-side callers (session link,       |
|   | (tokio task, 127.0.0.1)   |   iMessage scanner) use the same HTTP     |
|   | openhuman_core domains    |                                           |
|   +---------------------------+                                           |
+---------------------------------------------------------------------------+
```

There is no sidecar binary. `core_process::CoreProcessHandle` owns a tokio task
running `openhuman_rpc::host::desktop`, so the core lives and dies with the
window. That task owns the process's one embed runtime; a restart waits for
the old task to drop it before spawning the next. The handle generates a 256-bit hex bearer per
launch (`generate_rpc_token`) and hands it to the embedded server in memory,
not through the environment. The frontend gets it back with the
`core_rpc_endpoint` command.

### Boot sequence

`main.rs` runs first and looks at `argv[1]`. It installs nothing itself:
every path boots through an `openhuman_rpc::host` entry, which connects the
TinyHumans backend transport (the core carries none of its own).

- `OpenHuman core <args>` goes to `run_core_from_args`, which calls
  `openhuman_rpc::host::cli` (the same entry as the `openhuman-core` binary:
  connected, with the JSON-RPC server behind `run` / `serve`). On Windows the process reattaches to the parent console
  first so output appears in the shell.
- `OpenHuman mcp` and `OpenHuman mcp-server` do the same, which makes the app
  binary a stdio MCP server for clients such as the Claude Code CLI.
- Anything else calls `openhuman::run()` and starts the GUI.

`run()` in `lib.rs` then does, in order:

1. Builds the Tauri context (on Windows it drops native decorations for the
   custom titlebar) and neutralizes a broken parent stderr
   (`stderr_panic_hook`).
2. Replaces Tauri's async runtime with `openhuman_rpc::embed::process::tokio_runtime()`,
   a multi-thread runtime with the core's `AGENT_WORKER_STACK_BYTES` stack
   size and `MAX_BLOCKING_THREADS`, so agent turns started from commands do
   not overflow the default stack.
3. Initializes Sentry (DSN from `OPENHUMAN_TAURI_SENTRY_DSN`) with
   `embed::process::sentry::client_options`: the shared `before_send` chain
   (dev-server fetch noise, the core's known transient classes, hostname
   stripping, secret scrubbing) with the signed-in user id from
   `session::peek_user_id` as the fallback. Then the stderr
   panic hook, file logging (`file_logging::init`), and the Linux display and
   WSL checks.
4. Single-instance guards that must run before any window exists. On Windows
   a named mutex makes a second launch forward its `openhuman://` URLs over a
   named pipe (`deep_link_ipc_windows`) and exit, and
   `process_recovery::reap_stale_openhuman_processes` clears a wedged
   previous GUI instance. On Linux a second launch forwards its URLs over a
   Unix socket (`deep_link_ipc`) and exits.
5. Builds the Tauri app: the macOS app menu, `tauri-plugin-single-instance`
   (skipped on Linux when the D-Bus session bus is unreachable, because the
   plugin panics there), the opener, `external_navigation`, deep-link,
   notification, global-shortcut and updater plugins, and managed state for
   hotkeys, the staged app update and the iMessage scanner registry.
6. `setup()`: points the core at the installer's [`bundled-modules/`](bundled-modules/) resource
   directory, verifies `openhuman://` registration (Windows registry
   read-back, Linux xdg-utils), drains deep links that arrived early, removes
   a stale macOS LaunchAgent in debug builds, then creates the
   `CoreProcessHandle` on `default_core_port()` (`OPENHUMAN_CORE_PORT`, default
   7788), exports `OPENHUMAN_CORE_RPC_URL`, installs the session host
   (`session::install`), restores window geometry (`window_state`), shows the
   main window, and on macOS schedules the iMessage scanner.
7. `RunEvent::Ready` sets up the tray. Tray creation is deferred to here
   because GTK is not ready earlier, and `setup_tray` is a no-op on Linux.

The core is not started in `setup()`. The frontend's boot gate calls
`start_core_process`, which calls `CoreProcessHandle::ensure_running` and
shows a notification if the server had to fall back to another port. The one
exception is daemon mode (`daemon` or `--daemon` on the command line), where
the window starts hidden and the core is started directly.

### Starting the embedded core

`ensure_running` is idempotent. If the preferred port is already taken it
probes the listener. A previous OpenHuman core (left by a crashed or sibling
dev build) is terminated and replaced, with a revalidation step that guards
against PID reuse. Anything else is logged and the server tries its fallback
bind range. `OPENHUMAN_CORE_REUSE_EXISTING=1` skips all of this and attaches to
whatever is listening, which is how you point the shell at an external
`openhuman-core run` for debugging. Readiness is a oneshot signal from the
server plus a poll, with a 60 second ceiling. Debug builds also write the
bearer to `<tmp>/openhuman-e2e-rpc-token` (mode 0600) for the E2E harness.

`restart_core_process`, `recover_port_conflict` and `force_quit_port_owner`
are the recovery commands the boot gate offers when startup fails.

### How the frontend reaches the core

```text
coreRpcClient.ts
   |
   | invoke('core_rpc_endpoint')  ->  active_rpc_endpoint()
   |                                    |
   |                    gateways on:    gateway::registry::current()
   |                    gateways off:   embedded url + rpc_token
   v
 rpcUrlNeedsShellRelay(url)?
   |  no (loopback or https)            yes (plain http to a LAN host)
   v                                    v
 fetch(url) from the webview        invoke('relay_http_rpc', {url, token, body})
                                        -> core_rpc::post_json_rpc (reqwest)
```

`core_rpc_endpoint` returns the URL and bearer from one snapshot so the
renderer can never pair one core's URL with another core's token.
`core_rpc_url` and `core_rpc_token` are the older split accessors. The webview
origin is a secure context, so a plain-`http` runtime on a LAN address is
blocked as mixed content; `relay_http_rpc` sends that request from Rust
instead and returns the upstream status and body verbatim. With the
`gateways` feature, `post_json_rpc` refuses to attach a bearer to plain HTTP
off loopback.

### Shutdown

Every clean exit path (`RunEvent::ExitRequested`, tray or menu quit,
`app_quit`) saves the main window geometry and calls
`perform_early_teardown_sync_once`. That stops the iMessage scanner, tears
down any provisioned gateway, and sends the core its terminate signal. After
Tauri's event loop returns, `process_kill::sweep_orphan_children` kills any
child process still parented to the app. On macOS closing the main window
hides the app instead of destroying the webview, so the dock and tray can
bring it back; on Windows closing the main window exits.

## Layout

Core lifecycle and RPC:

| Path | What it does |
| --- | --- |
| [`src/main.rs`](src/main.rs) | Binary entry. Installs the backend transport and routes `core`, `mcp` and GUI launches. |
| [`src/lib.rs`](src/lib.rs) | `run()`, `run_core_from_args()`, most `#[tauri::command]`s, tray, menu, the `generate_handler!` list and the run-event loop. |
| [`src/core_process.rs`](src/core_process.rs) | `CoreProcessHandle`: start, restart, shut down the embedded server; stale-listener takeover; port-conflict recovery. |
| [`src/core_rpc.rs`](src/core_rpc.rs) | `relay_http_rpc`, `post_json_rpc`, and helpers for shell-side calls to the embedded core (`core_rpc_url_value`, `apply_auth`). |
| [`src/process_kill.rs`](src/process_kill.rs) | Cross-platform TERM and force-kill helpers and the exit-time orphan sweep. |
| [`src/process_recovery.rs`](src/process_recovery.rs) | Finds and reaps OpenHuman processes left by hard exits. See [`src/process_recovery/`](src/process_recovery/README.md). |
| [`src/gateway/`](src/gateway/) | Routes the frontend to a core in a container, over SSH, or both (feature `gateways`). See [`src/gateway/`](src/gateway/README.md). |
| [`src/session/`](src/session/) | Login, logout and current user, via `openhuman-tinyhumans`. See [`src/session/`](src/session/README.md). |

Platform integration:

| Path | What it does |
| --- | --- |
| [`src/deep_link_ipc.rs`](src/deep_link_ipc.rs) | Linux: a second launch forwards `openhuman://` URLs to the primary over a Unix socket; the primary emits `deep-link://new-url`. |
| [`src/deep_link_ipc_windows.rs`](src/deep_link_ipc_windows.rs) | Windows: the same over a named pipe. |
| [`src/deep_link_registration_check.rs`](src/deep_link_registration_check.rs) | Windows: reads back the `HKCU\Software\Classes\openhuman` registration and logs a redacted health report. |
| [`src/loopback_oauth.rs`](src/loopback_oauth.rs) | One-shot `http://127.0.0.1:<port>/auth` listener used as the RFC 8252 OAuth redirect; emits `loopback-oauth-callback`. |
| [`src/external_navigation.rs`](src/external_navigation.rs) | Plugin that cancels top-level `http(s)` navigation away from the app origin and opens the URL in the default browser. |
| [`src/native_notifications/`](src/native_notifications/) | Notification permission and delivery, real on macOS. See [`src/native_notifications/`](src/native_notifications/README.md). |
| [`src/imessage_scanner/`](src/imessage_scanner/) | macOS: reads `chat.db` and ingests conversations into memory. See [`src/imessage_scanner/`](src/imessage_scanner/README.md). |
| [`src/dictation_hotkeys.rs`](src/dictation_hotkeys.rs), [`src/ptt_hotkeys.rs`](src/ptt_hotkeys.rs) | Global shortcut parsing and state for dictation and push-to-talk. |
| [`src/ptt_overlay.rs`](src/ptt_overlay.rs) | Borderless always-on-top window for push-to-talk, rendering the `/ptt-overlay` route. |
| [`src/mascot_native_window.rs`](src/mascot_native_window.rs), [`src/notch_window.rs`](src/notch_window.rs) | macOS: native NSPanel plus WKWebView hosts for the floating mascot and the notch activity pill. |
| [`src/window_state.rs`](src/window_state.rs) | Saves and restores main window position and size, with a DPI guard and work-area clamping. |
| [`src/directory_picker.rs`](src/directory_picker.rs) | Native folder chooser for the folder memory source. |
| [`src/workspace_paths.rs`](src/workspace_paths.rs) | Open, reveal or preview a file, only after resolving it inside the active workspace. |

Updates, reset and diagnostics:

| Path | What it does |
| --- | --- |
| [`src/app_update.rs`](src/app_update.rs) | Bounded retry policy for the updater download. The update commands themselves are in `lib.rs`. |
| [`src/local_data_reset.rs`](src/local_data_reset.rs) | `reset_local_data`: asks the core which paths to remove, shuts the core down so its file handles close, removes the active user's local data, and starts the core again. |
| [`src/reset_reboot_schedule.rs`](src/reset_reboot_schedule.rs) | Windows: schedules deletion at next reboot when files are locked during a reset. |
| [`src/file_logging.rs`](src/file_logging.rs) | Resolves the data dir and calls `openhuman_rpc::embed::process::init_for_embedded`; `reveal_logs_folder`, `logs_folder_path`. |
| [`src/stderr_panic_hook.rs`](src/stderr_panic_hook.rs) | Stops a closed parent stderr pipe from turning log writes into panics. |

Other commands:

| Path | What it does |
| --- | --- |
| [`src/artifact_commands.rs`](src/artifact_commands.rs) | Copies an agent artifact into the Downloads folder. |
| [`src/mcp_commands.rs`](src/mcp_commands.rs) | Locates the `openhuman-core` binary and opens an MCP client's config file for the user. |
| [`src/claude_code.rs`](src/claude_code.rs) | Opens a native terminal running `claude auth login` for the Claude Code provider. |

Configuration and packaging:

| Path | What it does |
| --- | --- |
| [`tauri.conf.json`](tauri.conf.json) | Windows, bundle resources (agent prompts and `bundled-modules`), updater and installer settings. |
| [`capabilities/`](capabilities/) | The capability granted to the `main` and `overlay` windows. See [`capabilities/`](capabilities/README.md). |
| [`permissions/`](permissions/) | App permission sets referenced by the capability. See [`permissions/`](permissions/README.md). |
| [`bundled-modules/`](bundled-modules/) | Installer resource directory for native module releases. Empty in git (only `.gitkeep`; everything else is ignored). Release builds fill it with [`scripts/release/stage-modules.mjs`](../../scripts/release/stage-modules.mjs), laid out as `<id>/<version>/<host_key>/<archive>`, and `setup()` hands it to `openhuman_rpc::embed::modules::set_bundled_releases_dir`. Its contents still pass the core's digest and TinyBus admission checks. On macOS, [`scripts/release/macos-bundled-modules.sh`](../../scripts/release/macos-bundled-modules.sh) signs and checks it. |
| [`build.rs`](build.rs) | Runs `tauri_build`, and empties `bundle.resources` for non-release builds. |
| [`profiling/`](profiling/) | Standalone CPU and RAM profiler for a running app. See [`profiling/`](profiling/README.md). |
| `Info.plist`, `entitlements.sidecar.plist`, `nsis-hooks.nsh`, `main.desktop`, `postinst`, `postrm` | Platform packaging files for macOS, the Windows installer, and Linux packages. |

## IPC commands

The authoritative list is the `tauri::generate_handler!` call near the end of
`run()` in [`src/lib.rs`](src/lib.rs). Grouped:

| Group | Commands |
| --- | --- |
| Core endpoint | `core_rpc_endpoint`, `core_rpc_url`, `core_rpc_token`, `relay_http_rpc`, `overlay_parent_rpc_url` |
| Core lifecycle | `start_core_process`, `restart_core_process`, `recover_port_conflict`, `force_quit_port_owner`, `process_diagnostics_list_owned` |
| Gateways (feature `gateways`) | `gateway_list`, `gateway_save`, `gateway_delete`, `gateway_activate`, `gateway_active`, `gateway_status` |
| Session | `auth_login_with_token`, `auth_store_session`, `auth_logout`, `auth_state`, `auth_current_user`, `get_active_user_id` |
| Updates | `check_app_update`, `download_app_update`, `install_app_update`, `apply_app_update`, `check_core_update`, `apply_core_update` |
| App and windows | `app_quit`, `restart_app`, `activate_main_window`, `set_titlebar_for_sidebar`, `mascot_window_show`, `mascot_window_hide`, `notch_window_show`, `notch_window_hide` |
| Hotkeys and voice | `register_dictation_hotkey`, `unregister_dictation_hotkey`, `register_ptt_hotkey`, `unregister_ptt_hotkey`, `show_ptt_overlay` |
| Notifications | `notification_permission_state`, `notification_permission_request`, `show_native_notification` |
| Files and logs | `open_workspace_path`, `reveal_workspace_path`, `preview_workspace_text`, `download_artifact_to_downloads`, `pick_directory_via_dialog`, `reveal_logs_folder`, `logs_folder_path`, `reset_local_data` |
| Integrations | `start_loopback_oauth_listener`, `stop_loopback_oauth_listener`, `mcp_resolve_binary_path`, `mcp_open_client_config`, `claude_code_login_launch` |

`check_core_update` always reports the running version as current and
`apply_core_update` always errors: the core ships inside the app, so the app
updater is the only update path. Both remain for frontend compatibility.

Events the shell emits to the renderer include `auth://changed`,
`auth://expired`, `deep-link://new-url`, `loopback-oauth-callback`,
`app-update:status` and `app-update:progress`.

## Key types and entry points

- `openhuman::run()` (`src/lib.rs`) starts the GUI.
- `openhuman::run_core_from_args(args)` (`src/lib.rs`) runs the core CLI
  in-process.
- `CoreProcessHandle` ([`src/core_process.rs`](src/core_process.rs)) is managed Tauri state for the
  embedded server: `ensure_running`, `restart`, `shutdown`,
  `send_terminate_signal`, `rpc_url`, `rpc_token`, `port`.
- `active_rpc_endpoint` (`src/lib.rs`) is the single answer to "where does RPC
  go"; the endpoint commands and the session link both call it.
- `SessionHost` ([`src/session/mod.rs`](src/session/mod.rs)) is managed state wrapping the
  `openhuman_tinyhumans::SessionManager`.
- `gateway::registry::current` and `gateway::registry::activate`
  ([`src/gateway/registry.rs`](src/gateway/registry.rs)) hold the active gateway.

## Building

This crate is excluded from the root workspace. It has its own `Cargo.lock`
and resolves GTK, WebKit and Tauri only when you build it explicitly:

```bash
cargo check --manifest-path crates/openhuman-app/Cargo.toml
pnpm dev:app      # Vite dev server + this crate
pnpm build        # production bundle
```

The only OpenHuman dependency is `openhuman-rpc = { path = "../openhuman-rpc",
default-features = false, features = [...] }`
(`scripts/ci/check-crate-chain.mjs` enforces it); it forwards each gate down
the chain to the core. Because default features are off, every product gate
must be listed by hand:
`channels`, `media`, `inference`, `voice`, `web3`, `documents`, `modules`,
`flows`, `skills`, `mcp`, `crash-reporting`, `http-server`, `scheduler-gate`,
`file-logging`, `hosting`. A gate missing here disappears from
the shipped app with no build error. [`scripts/ci/check-feature-forwarding.mjs`](../../scripts/ci/check-feature-forwarding.mjs)
compares the list with [`scripts/ci/product-features.txt`](../../scripts/ci/product-features.txt), and `lib.rs` has two
`const _: () = assert!(...)` guards (`VOICE_COMPILED_IN`,
`HTTP_SERVER_COMPILED_IN`) that fail the build if `voice` or `http-server` is
dropped.

The same line turns on rpc's own `http-client` and `server` (the relay and the
embedded server) and `jev` (the Jev ranker). The session owner, the embed
facades and the backend transport come through it as
`openhuman_rpc::tinyhumans` and `openhuman_rpc::embed`. The `tinybox-*` crates
sit behind `gateways`.

The `[patch]` tables mirror the root [`Cargo.toml`](Cargo.toml) for `tinytools`, the
`tinyinference-*` crates, `tinyflows` and `tinychannels`. Keep them in sync:
drift makes Cargo resolve two copies of the same crate, and their types stop
being interchangeable (a shell `dyn Tool` would no longer be the core's).

Release profile settings (thin LTO, 16 codegen units, stripped symbols,
`debug = "line-tables-only"`) must match the root manifest.

### Feature flags

These are shell-local and unrelated to the core feature forwarding above.

| Feature | Meaning |
| --- | --- |
| `gateways` (default) | Compiles in [`src/gateway/`](src/gateway/) and the `tinybox-*` crates. Off, the gateway commands are absent and `active_rpc_endpoint` always answers with the embedded core. |
| `custom-protocol` | Serves the bundled `frontendDist` from `tauri://localhost` instead of the Vite `devUrl`. `cargo tauri build` turns it on; never add it to `default`. |
| `e2e-test-support` | Forwards `openhuman-rpc/e2e-test-support` (down the chain to the core) to expose `openhuman.test_reset`. The E2E build ([`app/scripts/e2e-build.sh`](../../app/scripts/e2e-build.sh)) enables it. |

## Boundaries

- Business rules, persistence, agents, memory and RPC methods belong to
  [`crates/openhuman-core`](../openhuman-core/). The shell orchestrates and presents; it does not
  duplicate core policy.
- The JSON-RPC server, envelopes and HTTP client live in [`crates/openhuman-rpc`](../openhuman-rpc/).
- Login-token exchange, `/auth/me`, the user cache and the backend transport
  live in [`crates/openhuman-tinyhumans`](../openhuman-tinyhumans/). The shell only exposes them as
  commands and forwards events.
- Box provisioning, SSH reach and Docker confinement are implemented in the
  [`vendor/tinybox`](../../vendor/tinybox/) submodule (`tinyhumansai/tinybox`).
- No JavaScript injection into child webviews. New behavior goes into Rust IPC
  hooks, and new Tauri plugins must be audited for `js_init_script`.
- The app runs on Wry. Do not reintroduce CEF or CDP-scanner assumptions. The
  iMessage scanner is separate because it reads `chat.db` directly.

## Gotchas

- Many comments and log lines in `lib.rs`, `deep_link_ipc*.rs` and
  `process_recovery.rs` still say "CEF" (`cef::shutdown`, "pre-CEF mutex",
  the `com.openhuman.app-cef-init` mutex name). They describe ordering that
  still matters (single-instance checks before any webview starts), not a CEF
  runtime.
- The iMessage scanner and other shell-side callers that use
  `core_rpc::core_rpc_url_value` always reach the embedded core through
  `OPENHUMAN_CORE_RPC_URL`, even when a gateway is active. The session link
  goes through `active_rpc_endpoint` and follows the gateway.
- Registering a command in `generate_handler!` is not enough. Because this
  crate defines app permissions in [`permissions/`](permissions/), Tauri enforces the ACL on
  app commands too, and the webview gets "not allowed. Command not found" for
  any command no granted permission lists. Add every new command to a
  permission file. At the time of writing several registered commands are in
  no permission file (the `gateway_*` commands, `mcp_*`,
  `claude_code_login_launch`, `mascot_window_*`, `set_titlebar_for_sidebar`,
  `recover_port_conflict`, `force_quit_port_owner`, `check_core_update`,
  `apply_core_update`, `process_diagnostics_list_owned`,
  `overlay_parent_rpc_url`). See [`permissions/`](permissions/README.md).
- Tests for this crate do not run under `pnpm test:rust`, which only covers the
  root workspace.

## Tests

Tests are `*_tests.rs` siblings of each module (and under
[`src/process_recovery/`](src/process_recovery/) and [`src/native_notifications/macos/`](src/native_notifications/macos/) for modules
defined inline). Run them directly, as CI does
([`.github/workflows/test-reusable.yml`](../../.github/workflows/test-reusable.yml)):

```bash
cargo test --manifest-path crates/openhuman-app/Cargo.toml
cargo test --manifest-path crates/openhuman-app/Cargo.toml gateway::
```

Desktop E2E specs live under [`app/test/e2e/specs/`](../../app/test/e2e/specs/).

## Further reading

- [`crates/openhuman-app/capabilities/README.md`](capabilities/README.md): the capabilities module README.
- [`crates/openhuman-app/profiling/README.md`](profiling/README.md): the profiling module README.
- [`gitbooks/developing/architecture/tauri-shell.md`](../../gitbooks/developing/architecture/tauri-shell.md): the Tauri shell.
- [`gitbooks/developing/architecture.md`](../../gitbooks/developing/architecture.md): architecture overview.
- [`gitbooks/developing/architecture/frontend.md`](../../gitbooks/developing/architecture/frontend.md): the frontend.
- [`gitbooks/developing/building-rust-core.md`](../../gitbooks/developing/building-rust-core.md): building the Rust core.
- [`gitbooks/developing/tinyhumans-api-key.md`](../../gitbooks/developing/tinyhumans-api-key.md): running on a TinyHumans API key.
- [`gitbooks/developing/performance.md`](../../gitbooks/developing/performance.md): performance.
- [`crates/openhuman-app/permissions/README.md`](permissions/README.md): the permissions module README.
