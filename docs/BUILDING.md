# Building & Installing OpenHuman

This guide covers two paths:

1. Build and compile OpenHuman from source
2. Install the latest stable release binaries

## Prerequisites

- `git`
- `node` + `yarn`
- Rust toolchain (see `rust-toolchain.toml`)

## Build from source (local compile)

Run from the repository root:

```bash
# 1) Clone and enter the repo
git clone https://github.com/tinyhumansai/openhuman.git
cd openhuman

# 2) Install JS deps (workspace)
yarn install

# 3) Build Rust core binary
cargo build --manifest-path Cargo.toml --bin openhuman

# 4) Stage core sidecar for the desktop app
cd app
yarn core:stage

# 5) Build desktop app artifacts
yarn build
```

For local development instead of production build:

```bash
yarn dev
```

## Install latest stable release (macOS/Linux)

Primary install command:

```bash
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash
```

Installer behavior:

- Resolves latest stable OpenHuman release for your platform
- Validates artifact digest when available
- Installs locally (no sudo by default)
- macOS: installs `OpenHuman.app` into `~/Applications`
- Linux: installs AppImage as `~/.local/bin/openhuman` and writes a desktop entry

Useful flags:

```bash
# Preview actions without writing files
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash -s -- --dry-run
```

## Windows (latest stable)

Use PowerShell:

```powershell
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

Windows installer behavior:

- Resolves latest stable release
- Downloads MSI/EXE for x64
- Verifies digest when available
- Runs per-user install where supported by installer package

Manual download links (all platforms):

- Website: https://tinyhuman.ai/openhuman
- Latest release: https://github.com/tinyhumansai/openhuman/releases/latest

## Troubleshooting

### macOS: `yarn dev:app` exits with "CEF cache is held by another OpenHuman instance"

**Symptom**

`yarn dev:app` (or any debug build of the Tauri shell) exits before the window appears with a message like:

```
[openhuman] CEF cache at /Users/<you>/Library/Caches/com.openhuman.app/cef is held by another OpenHuman instance (host <hostname>, pid 12345).
Quit the running instance and try again.
Workaround:
  pkill -f "OpenHuman.app/Contents"
  pkill -f "openhuman-core"
```

**Cause**

CEF (Chromium Embedded Framework) holds an exclusive lock on its user-data directory via a `SingletonLock` symlink under `~/Library/Caches/com.openhuman.app/cef`. Both the installed `.app` bundle and the dev binary use the same identifier (`com.openhuman.app`), so they cannot run side-by-side. Without the preflight, `cef::initialize` returns failure and the vendored `tauri-runtime-cef` panics with a Rust backtrace and no actionable message (this was issue #864 before the preflight landed).

**Fix**

Quit the other OpenHuman instance and re-run. Fastest path:

```bash
pkill -f "OpenHuman.app/Contents"
pkill -f "openhuman-core"
yarn dev:app
```

If the lock is left behind by a crashed process (PID no longer alive), the preflight removes the stale `SingletonLock` automatically and dev startup proceeds — no manual cleanup required.

**Known limitation**

Dev and release builds still share `com.openhuman.app` as the cache identifier. Isolating dev to a separate `com.openhuman.app.dev` cache requires changes to the vendored `tauri-runtime-cef` (cache path is built inside the runtime from the bundle identifier, not exposed to the openhuman shell). Tracked as a follow-up to #864.
