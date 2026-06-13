---
description: Build the Rust core from scratch on a fresh machine.
icon: terminal
---

# Building the Rust Core

This page is the contributor-facing reference for compiling the Rust core on a fresh machine.

It covers the **repo-root crate only**:

- Cargo package: `openhuman`
- Binary: `openhuman-core`
- Library: `openhuman_core`

If you want the full desktop app (`pnpm dev`, Tauri, CEF, frontend tooling), use [Getting Set Up](getting-set-up.md). That path has extra JavaScript, submodule, and desktop-runtime requirements that are **not** needed for a core-only `cargo` workflow.

## 1. Install the pinned Rust toolchain

The repository pins Rust in [`rust-toolchain.toml`](../../rust-toolchain.toml):

- Channel: `1.93.0`
- Components: `rustfmt`, `clippy`

Recommended install:

```bash
rustup toolchain install 1.93.0 --component rustfmt --component clippy
rustup default 1.93.0
```

You can also let `cargo` auto-install from `rust-toolchain.toml` after `rustup` itself is installed.

## 2. Clone the repo

Core-only work:

```bash
git clone https://github.com/tinyhumansai/openhuman.git
cd openhuman
```

That is enough for the root crate.

Desktop/Tauri work is different:

- `app/src-tauri/vendor/` submodules are only needed when building the desktop shell or CEF-aware Tauri tooling.
- For that flow, follow [Getting Set Up](getting-set-up.md) and run `git submodule update --init --recursive`.

## 3. Build commands

From the repository root:

```bash
# Fast dependency + type check
cargo check --manifest-path Cargo.toml

# Debug build of the actual CLI / RPC binary
cargo build --manifest-path Cargo.toml --bin openhuman-core

# Release build
cargo build --manifest-path Cargo.toml --release --bin openhuman-core

# Rust tests
cargo test --manifest-path Cargo.toml
```

Notes:

- The **package** name is `openhuman`, but the runnable binary is **`openhuman-core`**.
- If you prefer package-oriented cargo commands for packager scripts, use `-p openhuman`.
- The built binary lands at `target/debug/openhuman-core` or `target/release/openhuman-core`.

## 4. macOS prerequisites

Install:

- Xcode Command Line Tools: `xcode-select --install`

Why:

- `whisper-rs` compiles native code during the build.
- On macOS this crate is built with the `metal` feature enabled in [`Cargo.toml`](../../Cargo.toml), so Apple toolchains and SDK headers need to be present.

After Xcode CLT is installed, the core should build with the cargo commands above.

## 5. Linux prerequisites

### Core-only package set

Install these packages before running `cargo` on a fresh Linux machine.

**Ubuntu / Debian:**

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential cmake pkg-config clang libssl-dev libclang-dev \
  libasound2-dev libxi-dev libxtst-dev libxdo-dev libudev-dev \
  libstdc++-14-dev
```

**Arch Linux:**

```bash
sudo pacman -S --needed base-devel cmake pkgconf clang openssl \
  alsa-lib libxi libxtst xdotool libevdev
```

> On Arch, `clang` includes `libclang` and `base-devel` includes `gcc` (providing `libstdc++`), so separate `-dev` packages are not needed.

Why these matter:

- `build-essential` / `base-devel`, `cmake`, `pkg-config` / `pkgconf`: native builds used by transitive Rust dependencies.
- `clang`, `libclang-dev`: bindgen / C and C++ compilation paths used by native crates.
- `libssl-dev` / `openssl`: OpenSSL headers needed by some networking dependencies.
- `libasound2-dev` / `alsa-lib`, `libxi-dev` / `libxi`, `libxtst-dev` / `libxtst`, `libxdo-dev` / `xdotool`, `libudev-dev` (included in Arch `systemd-libs`), `libevdev`: required by audio/input/device crates pulled into the core build.

### `whisper-rs` + `clang` note

`whisper-rs-sys` can fail under `clang` with:

```text
fatal error: 'array' file not found
```

This is why the docs call out `libstdc++-14-dev`: `clang` may pick GCC 14 C++ headers on Ubuntu runners.

If your distro layout still leaves `libstdc++.so` unresolved for the build, use the same workaround documented in [`AGENTS.md`](../../AGENTS.md):

```bash
# Ubuntu/Debian — adjust the GCC version as needed
sudo ln -sf /usr/lib/gcc/x86_64-linux-gnu/13/libstdc++.so /usr/lib/x86_64-linux-gnu/libstdc++.so
```

Arch Linux typically does not need this workaround because `gcc-libs` places `libstdc++.so` on the default library search path.

### Linux desktop/Tauri package set

If you are building the desktop shell instead of the core-only crate, install the broader dependency set.

**Ubuntu / Debian** (mirrored from [`.github/workflows/build-desktop.yml`](../../.github/workflows/build-desktop.yml)):

```bash
sudo apt-get update
sudo apt-get install -y \
  libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  patchelf cmake libasound2-dev libxdo-dev libxtst-dev libx11-dev libxi-dev \
  libevdev-dev libssl-dev libclang-dev \
  libnss3 libnspr4 libatk1.0-0 libatk-bridge2.0-0 libcups2 libdrm2 \
  libxkbcommon0 libxcomposite1 libxdamage1 libxfixes3 libxrandr2 \
  libgbm1 libpango-1.0-0 libcairo2 libatspi2.0-0 libxshmfence1 libu2f-udev
```

**Arch Linux:**

```bash
sudo pacman -S --needed gtk3 webkit2gtk-4.1 libayatana-appindicator \
  librsvg patchelf nss nspr at-spi2-core libcups libdrm \
  libxkbcommon libxcomposite libxdamage libxfixes libxrandr \
  mesa pango cairo libxshmfence
```

Use the desktop lists only when you need `app/src-tauri/`; for root-crate work, the smaller core-only list above is the relevant baseline.

## 6. Windows prerequisites

Install:

- Rust via `rustup`
- Visual Studio Build Tools 2022 or Visual Studio with the **Desktop development with C++** workload
- The MSVC target used by CI and release builds: `x86_64-pc-windows-msvc`

Recommended commands after the Microsoft toolchain is installed:

```powershell
rustup toolchain install 1.93.0 --component rustfmt --component clippy
rustup target add x86_64-pc-windows-msvc
cargo build --manifest-path Cargo.toml --bin openhuman-core
```

Windows note:

- The repo patches `whisper-rs-sys` to force the static MSVC CRT and avoid the `LNK2038` / `LNK1169` mismatch called out in [`Cargo.toml`](../../Cargo.toml). Use the MSVC toolchain, not MinGW.

## 7. Related paths

- [Getting Set Up](getting-set-up.md): full desktop contributor setup with `pnpm`, Tauri, submodules, and sidecar staging.
- [OpenHuman Architecture](architecture/README.md): where the core fits into the desktop app and RPC flow.
