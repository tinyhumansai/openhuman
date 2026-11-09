---
description: >-
  Build OpenHuman from source, or install a release: toolchain, submodules,
  desktop builds and the installers for each platform.
icon: wrench
---

# Getting set up

There are two ways to get OpenHuman running: build it from source, or install the latest release. This page covers both.

If you only need the Rust workspace under `crates/` (no Node, no Tauri), use [Building the Rust core](building-rust-core.md). It has the pinned toolchain, OS packages and the `cargo` commands for `openhuman-core`.

## Prerequisites

- `git`
- Node.js 24 or newer (see `app/package.json`)
- `pnpm@10.10.0` (see the root `package.json` `packageManager` field)
- Rust 1.99.1 through `rustup` with `rustfmt` and `clippy` (see `rust-toolchain.toml`)
- CMake, required by native Rust dependencies
- Every `vendor/` submodule listed in `.gitmodules`, initialized recursively (`git submodule update --init --recursive`). The root `Cargo.toml` `[patch]` tables point into that tree, so cargo reads every manifest, optional crate or not. A partial checkout always fails to build
- Platform desktop build tools: Xcode Command Line Tools on macOS, or the Tauri GTK/WebKit/AppIndicator package set on Linux

macOS Homebrew quick start:

```bash
brew install node@24 pnpm rustup-init cmake
rustup toolchain install 1.99.1 --profile minimal
rustup component add rustfmt clippy --toolchain 1.99.1
```

Arch Linux quick start:

```bash
sudo pacman -S --needed nodejs npm rustup cmake base-devel clang openssl \
  alsa-lib xdotool libxtst libxi libevdev gtk3 webkit2gtk-4.1 \
  libayatana-appindicator librsvg patchelf nss nspr at-spi2-core \
  libcups libdrm libxkbcommon libxcomposite libxdamage libxfixes \
  libxrandr mesa pango cairo libxshmfence
npm install -g pnpm@10.10.0
rustup toolchain install 1.99.1 --profile minimal
rustup component add rustfmt clippy --toolchain 1.99.1
```

## Build from source

Run from the repository root:

```bash
# 1) Clone and enter the repo
git clone https://github.com/tinyhumansai/openhuman.git
cd openhuman

# 2) Fetch the vendored tiny* submodules
git submodule update --init --recursive

# 3) Install JS dependencies
pnpm install

# 4) Build the desktop app
pnpm build
```

For development instead of a production build:

```bash
# Web-only UI development
pnpm dev

# Desktop app development: runs scripts/run-dev-macos.sh (`cargo tauri dev` with a dev config override)
pnpm dev:app

# Other Tauri CLI commands (from app/node_modules) run against crates/openhuman-app/
pnpm tauri build
```

## Install the latest release on macOS and Linux

Run the install script:

```bash
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash
```

The script:

- Finds the latest stable release for your platform.
- Checks the download against its SHA-256 digest.
- Installs without sudo.
- On macOS, installs `OpenHuman.app` into `~/Applications`.
- On Linux x64, installs the AppImage as `~/.local/bin/openhuman` and writes a desktop entry.

To preview what it would do without writing files, add `--dry-run`:

```bash
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash -s -- --dry-run
```

### Arch Linux

There is no published AUR package. The repository includes an `openhuman-bin` recipe at [`packages/arch/openhuman-bin`](https://github.com/tinyhumansai/openhuman/tree/main/packages/arch/openhuman-bin). It uses the official x86_64 AppImage, extracts it during `makepkg`, installs a desktop entry and exposes `/usr/bin/openhuman`. Build it locally:

```bash
cd packages/arch/openhuman-bin
makepkg --syncdeps --install
```

## Windows

Use PowerShell:

```powershell
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

The script:

- Finds the latest stable release.
- Downloads the x64 MSI or EXE.
- Checks its SHA-256 digest.
- Runs a per-user install where the installer supports it.

## ARM Linux build (aarch64)

CI builds the `aarch64-unknown-linux-gnu` target on an `ubuntu-24.04-arm` runner with the same Tauri command as x64 (see
[`.github/workflows/build-desktop.yml`](https://github.com/tinyhumansai/openhuman/blob/main/.github/workflows/build-desktop.yml)).
Locally, with the Linux desktop package set installed:

```bash
pnpm tauri build --target aarch64-unknown-linux-gnu --bundles deb appimage
```

The shell is stock Tauri on Wry, so the binary needs no extra library path. Install the `.deb` bundle with `dpkg -i`.

You can also download any platform's installer from [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman) or the [latest release](https://github.com/tinyhumansai/openhuman/releases/latest).

## Troubleshooting

### Stale `openhuman` process on the core port

A previous Tauri build or `openhuman-core run` can leave a process listening on `OPENHUMAN_CORE_PORT` (default `7788`). If a new build attached to it, you would see version drift and 401s, because the new build's `OPENHUMAN_CORE_TOKEN` would not match.

`core_process::ensure_running` now probes the port at startup:

- If `GET /` identifies the listener as an OpenHuman core (a JSON body with `"name": "openhuman"`), it is treated as a stale process and stopped. On Unix it gets `SIGTERM`, then `SIGKILL` after 750 ms. On Windows it uses `taskkill /F /T /PID`. The Tauri host then starts its own embedded core.
- If the listener is something else, or does not speak HTTP, startup fails and the log shows the conflict.
- Set `OPENHUMAN_CORE_REUSE_EXISTING=1` to attach to whatever is listening instead. This is useful when you run `openhuman-core run` by hand for debugging.

To clean up manually:

```bash
pkill -f "OpenHuman.app/Contents"
pkill -f "openhuman-core"
```

---

## See also

- [Building the Rust core](building-rust-core.md): the cargo-only path, without Node or Tauri.
- [E2E testing guide](e2e-testing.md): the harness a local desktop build feeds.
- [Architecture](architecture.md): what the shell, the core and the frontend each own.
- [Cloud deployment](../features/cloud-deploy.md): running the same core headless on a server.
