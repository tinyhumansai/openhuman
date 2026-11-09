---
description: Build the Rust core from scratch on a fresh machine.
icon: terminal
---

# Building the Rust core

This page shows how to compile the Rust core on a fresh machine. It covers the core workspace and its sibling crates:

- Cargo package: `openhuman`
- Binary: `openhuman-core` (built from `openhuman-cli`)
- Library: `openhuman_core`

The root `Cargo.toml` is a virtual workspace whose members are
`crates/openhuman-core`, `crates/openhuman-embed`, `crates/openhuman-rpc`,
`crates/openhuman-tinyhumans`, `crates/openhuman-cli`, and
`crates/openhuman-tui`. `crates/openhuman-app` (the Tauri desktop shell) is
excluded from that workspace and builds from its own manifest.

If you want the full desktop app (`pnpm dev`, Tauri, frontend tooling), use [Getting set up](getting-set-up.md). That path needs extra JavaScript, submodule and desktop-runtime tooling that a core-only `cargo` workflow does not.

## 1. Install the pinned Rust toolchain

The repository pins Rust in [`rust-toolchain.toml`](https://github.com/tinyhumansai/openhuman/blob/main/rust-toolchain.toml):

- Channel: `1.99.1`
- Components: `rustfmt`, `clippy`

The pin exists because `rusqlite` 0.40 and `libsqlite3-sys` 0.38 use the
`cfg_select!` macro, which became stable in 1.96.

Recommended install:

```bash
rustup toolchain install 1.99.1 --component rustfmt --component clippy
rustup default 1.99.1
```

You can also let `cargo` auto-install from `rust-toolchain.toml` after `rustup` itself is installed.

## 2. Clone the repo

Core-only work:

```bash
git clone https://github.com/tinyhumansai/openhuman.git
cd openhuman
```

That is enough for the Rust workspace. Here is where things live:

- `crates/openhuman-core/` holds the core sources, the package manifest and the domain implementation.
- `crates/openhuman-embed/` is the library facade for hosts.
- `crates/openhuman-tui/` is the terminal frontend.
- `crates/openhuman-cli/` holds the `openhuman-core` binary, the developer and benchmark bins, and the root `tests/*.rs` and `examples/*.rs` targets. It depends on `crates/openhuman-tinyhumans/` for the SDK-backed backend transport, which the core does not carry.
- `crates/openhuman-rpc/` holds the JSON-RPC contracts and the HTTP client that the Tauri shell and the TUI use.

The recursive submodules under repo-root `vendor/` are required for the core
build too, not just the desktop shell. `crates/openhuman-core/Cargo.toml`
path-depends on `vendor/tinyagents`, `vendor/tinymemory`, `vendor/tinymcp`,
and the rest of the `tiny*` family, and the root `Cargo.toml` `[patch]`
tables point into `vendor/tinymemory`, `vendor/tinyflows`,
`vendor/tinychannels`, and the `tinyinference`
copy nested under `vendor/tinyagents/`.

```bash
git submodule update --init --recursive vendor/
```

Desktop and Tauri work has extra requirements. Follow [Getting set up](getting-set-up.md) for those.

## 3. Build commands

From the repository root:

```bash
# Fast dependency and type check
cargo check --manifest-path Cargo.toml

# Debug build of the CLI and RPC binary
cargo build --manifest-path Cargo.toml -p openhuman-cli --bin openhuman-core

# Check the stable host-facing embedding facade
cargo check --manifest-path Cargo.toml -p openhuman-embed

# Check the shared RPC contracts and HTTP client crate
cargo check --manifest-path Cargo.toml -p openhuman-rpc

# Build the terminal frontend (embeds the core in-process)
cargo build --manifest-path Cargo.toml -p openhuman-tui

# Check the desktop shell (separate Cargo world, own manifest and lockfile)
cargo check --manifest-path crates/openhuman-app/Cargo.toml

# Release build
cargo build --manifest-path Cargo.toml --release -p openhuman-cli --bin openhuman-core

# Rust tests
cargo test --manifest-path Cargo.toml
```

Notes:

- The core library's package is `openhuman` (crate `openhuman_core`). The runnable binary, `openhuman-core`, is built from the `openhuman-cli` package (`crates/openhuman-cli/src/main.rs`), which is why the commands above pass `-p openhuman-cli`.
- The built binary lands at `target/debug/openhuman-core` or `target/release/openhuman-core`.

### Faster local linking (optional)

The `openhuman` core crate links a large single rlib, so the edit, `cargo check` and `cargo test` loop is often link-bound. A faster linker (mold
on Linux, lld on macOS) cuts a large slice off every incremental relink.
[`.cargo/config.toml`](https://github.com/tinyhumansai/openhuman/blob/main/.cargo/config.toml) documents the manual
opt-in. The easiest path is:

```bash
# installs mold/lld detection into $CARGO_HOME/config.toml, never the
# repo's tracked .cargo/config.toml, so it is a per-machine opt-in
scripts/dev-setup-linker.sh

# preview the change first
scripts/dev-setup-linker.sh --dry-run
```

Install the linker first (`apt install mold` / `brew install llvm`). The
script detects it and exits with instructions if it is missing. It is
idempotent, so running it again after the linker is configured does nothing.
CI enables the same flag through `RUSTFLAGS` in the Linux Rust jobs. The
script gives local `cargo` runs the same speedup.

### Feature gates and binary size

`crates/openhuman-core/Cargo.toml` builds most of its domains behind Cargo
features. A bare `cargo check` compiles the contributor default set (`media`,
`skills`, `flows`, `mcp`, `channels`, `http-server`, `scheduler-gate`,
`file-logging`, `modules`), which is not what the desktop app
ships. `scripts/ci/product-features.txt` is the single source of truth for
the shipped product's gate list, and `scripts/ci/check-feature-forwarding.mjs`
checks that `crates/openhuman-app/Cargo.toml` forwards exactly that set.
Turning a gate off drops real code and, for some gates, whole native dependencies. For measured binary sizes and RSS across a few feature recipes,
see [Performance](performance.md).

## Local module development

Loadable modules (`documents`, `voice`, `mcp`, and so on) come from pinned,
digest-checked GitHub releases by default. That is the only source CI and
shipped builds use. To iterate on a module in `vendor/<submodule>` without
releasing it, switch the source locally:

```bash
# Build the module from its gitlink and print the environment for it
eval "$(node scripts/modules/build.mjs tinydocs --print-env)"
OPENHUMAN_MODULE_SOURCE=source ./target/debug/openhuman-core serve

# Or link it into the CLI (non-product feature)
OPENHUMAN_MODULE_SOURCE=linked cargo run -p openhuman-cli --features linked-modules -- serve

# Before pushing: is the gitlink ahead of the registry pin?
node scripts/modules/pin-status.mjs

# Release the module and re-pin the registry (asks before publishing)
node scripts/modules/release-and-repin.mjs tinydocs --bump patch
```

`linked-modules` links all eleven registry modules (`crates/openhuman-modules-linked`, one
feature per module plus `all`), so `OPENHUMAN_MODULE_SOURCE=linked` serves any of them from the
executable. They share one `tinybus`, one TinyAgents and one `tinystoragedrivers` through the root
`[patch]` tables, which is why a module repo names those by version range and never by `=` pin or
nested path. Check it with `cargo build -p openhuman-modules-linked --features all`.

`OPENHUMAN_MODULE_SOURCE_<ID>` overrides one module. If a `linked` or `source`
module is unavailable the core falls back to the release and logs a warning;
`modules.list` shows which source each module is served from. Details:
[Loadable modules](loadable-modules.md).

## 4. macOS prerequisites

Install:

- Xcode Command Line Tools: `xcode-select --install`

Why: native dependencies compile C and Objective-C code during the build, and they need the Apple toolchain and SDK headers. These include `cpal` for audio capture (behind the `inference` feature, which `voice` requires) and the `objc2` Contacts crates compiled by the vendored `tinymemory` crates.

After Xcode CLT is installed, the core should build with the cargo commands above.

## 5. Linux prerequisites

### Core-only package set

Install these packages before running `cargo` on a fresh Linux machine.

Ubuntu or Debian:

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential cmake pkg-config clang libssl-dev libclang-dev \
  libasound2-dev libxi-dev libxtst-dev libxdo-dev libudev-dev \
  libstdc++-14-dev
```

Arch Linux:

```bash
sudo pacman -S --needed base-devel cmake pkgconf clang openssl \
  alsa-lib libxi libxtst xdotool libevdev
```

> On Arch, `clang` includes `libclang` and `base-devel` includes `gcc` (providing `libstdc++`), so you do not need separate `-dev` packages.

Why these matter:

- `build-essential` / `base-devel`, `cmake`, `pkg-config` / `pkgconf`: native builds for transitive Rust dependencies.
- `clang`, `libclang-dev`: bindgen (used by native crates such as `cpal`'s ALSA bindings) and C/C++ compilation paths.
- `libssl-dev` / `openssl`: OpenSSL headers needed by some networking dependencies.
- `libasound2-dev` / `alsa-lib`, `libxi-dev` / `libxi`, `libxtst-dev` / `libxtst`, `libxdo-dev` / `xdotool`, `libudev-dev` (included in Arch `systemd-libs`), `libevdev`: required by the audio, input and device crates (`cpal`, `enigo`/X11 input handling) pulled into the core build.
- `libstdc++-14-dev`: `clang`-driven builds may pick GCC 14 C++ headers on Ubuntu runners; this keeps `libstdc++.so` resolvable for those native crates.

### Linux desktop/Tauri package set

If you are building the desktop shell instead of the core-only crate, install the broader dependency set.

Ubuntu or Debian (mirrored from [`.github/workflows/build-desktop.yml`](https://github.com/tinyhumansai/openhuman/blob/main/.github/workflows/build-desktop.yml)):

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

Arch Linux:

```bash
sudo pacman -S --needed gtk3 webkit2gtk-4.1 libayatana-appindicator \
  librsvg patchelf nss nspr at-spi2-core libcups libdrm \
  libxkbcommon libxcomposite libxdamage libxfixes libxrandr \
  mesa pango cairo libxshmfence
```

Use the desktop lists only when you need `crates/openhuman-app/`; for root-crate work, the smaller core-only list above is the relevant baseline.

## 6. Windows prerequisites

Install:

- Rust via `rustup`
- Visual Studio Build Tools 2022 or Visual Studio with the Desktop development with C++ workload
- The MSVC target used by CI and release builds: `x86_64-pc-windows-msvc`

Recommended commands after the Microsoft toolchain is installed:

```powershell
rustup toolchain install 1.99.1 --component rustfmt --component clippy
rustup target add x86_64-pc-windows-msvc
cargo build --manifest-path Cargo.toml -p openhuman-cli --bin openhuman-core
```

Use the MSVC toolchain, not MinGW, to match CI and release builds.

## 7. Related paths

- [Getting set up](getting-set-up.md): the full desktop contributor setup with `pnpm`, Tauri and submodules. The core runs in-process inside the desktop shell (see [Tauri shell](architecture/tauri-shell.md)), so there is no sidecar staging step.
- [Architecture overview](architecture/README.md): where the core fits into the desktop app and RPC flow.
- [Architecture](architecture.md): the full crate map and repository layout.
- [Embedding](embedding.md): using `crates/openhuman-embed` to run the core as a library in another product.
- [Engines](engines.md): the pluggable LLM, embedding, memory and search backends the core can run against.
- [Performance](performance.md): binary size, cold start, and memory numbers across feature recipes.
