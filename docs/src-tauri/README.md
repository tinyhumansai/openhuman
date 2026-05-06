# Tauri shell documentation (`app/src-tauri/`)

This folder is the **desktop host** for OpenHuman: Tauri v2 + WebView, IPC commands, window management, and bridging to the **`openhuman`** Rust sidecar (core JSON-RPC). It does **not** duplicate the full domain stack — that lives in the repo-root Rust crate (`openhuman_core`, `src/bin/openhuman.rs`).

## Quick reference

| Document                             | Description                                |
| ------------------------------------ | ------------------------------------------ |
| [Architecture](./01-architecture.md) | Modules, core process, sidecar staging     |
| [IPC commands](./02-commands.md)     | `invoke` commands registered in `lib.rs`   |
| [Core bridge](./03-services.md)      | `core_process`, `core_rpc`, daemon helpers |

## Responsibilities

1. **Web UI** — Load the Vite build from `app/dist` (or dev server on port 1420).
2. **IPC** — Expose a small, explicit set of Tauri commands (see `02-commands.md`).
3. **Core lifecycle** — Ensure the **`openhuman`** binary is running (child process and/or service) and proxy JSON-RPC via **`core_rpc_relay`**.
4. **AI prompts on disk** — Resolve bundled `src/openhuman/agent/prompts` from resources / dev cwd for `ai_get_config` / `write_ai_config_file`.
5. **Window + tray** — Desktop window behavior and system tray (see `lib.rs`).

## Building the sidecar

`app/package.json` **`core:stage`** runs `scripts/stage-core-sidecar.mjs`, which `cargo build --bin openhuman` at the repo root and copies the binary into `app/src-tauri/binaries/` for Tauri `externalBin`.

## Stuck process recovery

Normal app quit runs teardown from `RunEvent::ExitRequested`: child webviews are closed before CEF shutdown, the embedded core's cancellation token is triggered, and the final process sweep sends `SIGTERM` to direct children before escalating holdouts with `SIGKILL` after a short grace period. Sweep summaries are logged as `[app] sweep: term=N kill=M total=K`; any nonzero `kill` count is a warning and means a child ignored graceful shutdown.

On macOS, hard exits such as Force Quit, `SIGKILL`, or a renderer crash can skip normal teardown. The next launch runs startup recovery before CEF cache preflight: it lists OpenHuman processes whose executable path belongs to the launching `.app/Contents`, skips the current process, sends `SIGTERM`, waits briefly, then sends `SIGKILL` to stragglers that still match the same pid and command. These events use the `[startup-recovery]` log prefix.

Startup recovery intentionally skips when `OPENHUMAN_CORE_REUSE_EXISTING=1` is set so manual CLI-core reuse still works. It also skips when the CEF `SingletonLock` is held by a live process, letting the normal second-instance path fail without killing the already-running app.

For diagnostics, the Tauri command `process_diagnostics_list_owned` returns the currently owned process list used by startup recovery, or an error if process enumeration fails. The macOS implementation is bundle-scoped; Linux and Windows currently return an empty list.

## Related

- Full stack narrative: [`../ARCHITECTURE.md`](../ARCHITECTURE.md)
- Frontend: [`../src/README.md`](../src/README.md)
