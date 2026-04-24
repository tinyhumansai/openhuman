# Tauri IPC commands (`app/src-tauri`)

All commands are registered in **`app/src-tauri/src/lib.rs`** inside `tauri::generate_handler![...]` (desktop build). Names below are the **Rust** command names (camelCase in JS via serde where applicable).

## Demo / diagnostics

| Command | Purpose                                    |
| ------- | ------------------------------------------ |
| `greet` | Demo string (safe to remove in production) |

## AI configuration (bundled prompts)

| Command                | Purpose                                                                                      |
| ---------------------- | -------------------------------------------------------------------------------------------- |
| `ai_get_config`        | Build `AIPreview` from resolved `SOUL.md` / `TOOLS.md` under bundled or dev `src/openhuman/agent/prompts` |
| `ai_refresh_config`    | Same read path as `ai_get_config` (refresh hook)                                             |
| `write_ai_config_file` | Write a single `.md` under repo `src/openhuman/agent/prompts` (dev / safe filename checks)                |

## Core JSON-RPC relay

| Command          | Purpose                                                                                                        |
| ---------------- | -------------------------------------------------------------------------------------------------------------- |
| `core_rpc_relay` | Body: `{ method, params?, serviceManaged? }` → forwards to local **`openhuman`** HTTP JSON-RPC (`core_rpc.rs`) |

Use **`app/src/services/coreRpcClient.ts`** (`callCoreRpc`) from the frontend.

## Window management

From **`commands/window.rs`** (names may vary slightly; see `lib.rs`):

| Command             | Purpose           |
| ------------------- | ----------------- |
| `show_window`       | Show main window  |
| `hide_window`       | Hide main window  |
| `toggle_window`     | Toggle visibility |
| `is_window_visible` | Query visibility  |
| `minimize_window`   | Minimize          |
| `maximize_window`   | Maximize          |
| `close_window`      | Close             |
| `set_window_title`  | Set title string  |

## OpenHuman daemon / service helpers

From **`commands/openhuman.rs`** (see source for exact payloads):

| Command                            | Purpose                                        |
| ---------------------------------- | ---------------------------------------------- |
| `openhuman_get_daemon_host_config` | Read daemon host preferences (e.g. tray)       |
| `openhuman_set_daemon_host_config` | Persist daemon host preferences                |
| `openhuman_service_install`        | Install background service (platform-specific) |
| `openhuman_service_start`          | Start service                                  |
| `openhuman_service_stop`           | Stop service                                   |
| `openhuman_service_status`         | Query status                                   |
| `openhuman_service_uninstall`      | Uninstall service                              |

## Screen share picker (CEF / macOS)

From **`screen_capture/mod.rs`** (registered under `#[cfg(feature = "cef")]`). Backs the in-page `getDisplayMedia` shim in `webview_accounts/runtime.js`. Session-gated: the shim must open a session with a live user gesture before enumeration / thumbnail captures succeed. See issue #713 (picker UX) + #812 (session gating).

| Command                           | Purpose                                                                                                                 |
| --------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `screen_share_begin_session`      | Open a 30s session from an account webview, after a `navigator.userActivation.isActive` gesture. Returns `{ token, sources }`. Rate-limited to 10/minute per account. |
| `screen_share_thumbnail`          | Capture a single source's thumbnail as base64 PNG. Requires a live token and an `id` that the session was issued for. macOS only; other platforms return an error.    |
| `screen_share_finalize_session`   | Close the session. Called by the shim on Share or Cancel; safe to call with an unknown/expired token (no-op).                                                         |

## Removed / not present

The following **do not** exist in the current `generate_handler!` list: `exchange_token`, `get_auth_state`, `socket_connect`, `start_telegram_login`. Authentication and sockets are handled in the **React** app and **core** process, not via these IPC names.

## Example: core RPC

```typescript
import { invoke } from "@tauri-apps/api/core";

const result = await invoke("core_rpc_relay", {
  request: {
    method: "your.rpc.method",
    params: { foo: "bar" },
    serviceManaged: false,
  },
});
```

---

_See `app/src-tauri/src/lib.rs` for the authoritative list._
