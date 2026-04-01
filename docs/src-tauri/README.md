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

## Updater boundaries

- **Tauri updater** (release `latest.json` + desktop bundle artifacts) updates the **desktop app bundle**.
- **Core self-update** (`openhuman.update_*` JSON-RPC controllers in the Rust core) updates only the **`openhuman-core` sidecar binary**.
- On app startup, the Tauri host attempts a **preflight sidecar swap** when a staged `*.next` core binary exists, then starts the child sidecar process.
- **Digest verification** is best-effort: if the release asset includes a `sha256:` digest or a companion checksum file, the updater verifies it; otherwise the download proceeds without digest validation. A follow-up issue should add companion `.sha256` files to the release pipeline for stronger integrity guarantees.

## Related

- Full stack narrative: [`../ARCHITECTURE.md`](../ARCHITECTURE.md)
- Frontend: [`../src/README.md`](../src/README.md)
