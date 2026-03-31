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
