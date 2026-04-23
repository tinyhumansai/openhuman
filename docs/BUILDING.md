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

## ARM Linux Build (aarch64)

The ARM Linux build requires special handling due to CEF and GTK dependencies.

### Prerequisites

```bash
# Install xvfb for headless builds/testing
sudo apt install xvfb
```

### Build

```bash
cd app
pnpm tauri build --target aarch64-unknown-linux-gnu
```

### Running the ARM binary

The binary requires the CEF library path to be set:

```bash
# Option 1: Direct invocation
CEF_DIR=app/src-tauri/target/aarch64-unknown-linux-gnu/release/build/cef-dll-sys-06f9a023be70e68b/out/cef_linux_aarch64
REL_DIR=app/src-tauri/target/aarch64-unknown-linux-gnu/release
LD_LIBRARY_PATH="$CEF_DIR:$REL_DIR/deps:$REL_DIR" $REL_DIR/OpenHuman --no-sandbox

# Option 2: Wrapper script (recommended)
# Create ~/bin/openhuman:
#!/bin/bash
CEF_DIR=/path/to/app/src-tauri/target/aarch64-unknown-linux-gnu/release/build/cef-dll-sys-06f9a023be70e68b/out/cef_linux_aarch64
REL_DIR=/path/to/app/src-tauri/target/aarch64-unknown-linux-gnu/release
export LD_LIBRARY_PATH="$CEF_DIR:$REL_DIR/deps:$REL_DIR"
exec $REL_DIR/OpenHuman --no-sandbox "$@"
```

### DEB package install

```bash
sudo dpkg -i app/src-tauri/target/aarch64-unknown-linux-gnu/release/bundle/deb/OpenHuman_0.52.28_arm64.deb
OpenHuman
```

### GTK initialization fix

The ARM build requires GTK to be initialized before Tauri creates the system tray. This is handled in `vendor/tauri-cef/crates/tauri-runtime-cef/src/lib.rs`:

```rust
// After CEF initialization, add:
#[cfg(target_os = "linux")]
{
    gtk::init().ok();
}
```

If the tray fails to initialize with "GTK has not been initialized", rebuild after ensuring this fix is in place.

Manual download links (all platforms):

- Website: https://tinyhuman.ai/openhuman
- Latest release: https://github.com/tinyhumansai/openhuman/releases/latest
