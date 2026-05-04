# Installing OpenHuman

## Quick install

| Package manager | Command | OS |
|---|---|---|
| **Homebrew** | `brew install tinyhumansai/openhuman/openhuman` | macOS, Linux |
| **apt** | `sudo apt install openhuman` (see [setup](#apt-debianubuntu)) | Debian, Ubuntu |
| **npm** | `npm install -g openhuman` | Any (Node ≥ 18) |
| **curl** | `curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh \| bash` | macOS, Linux |

---

## Homebrew (macOS / Linux)

```bash
brew install tinyhumansai/openhuman/openhuman
```

This is the current official Homebrew distribution channel and is backed by the
custom tap at
[tinyhumansai/homebrew-openhuman](https://github.com/tinyhumansai/homebrew-openhuman).
The repository also now tracks a `homebrew/core` source-formula candidate for a
future upstream submission; see [Homebrew Core Submission](./homebrew-core.md).

**Update:**
```bash
brew upgrade openhuman
```

**Uninstall:**
```bash
brew uninstall openhuman
brew untap tinyhumansai/openhuman   # optional: remove tap
```

Homebrew installs the binary as `openhuman`.

---

## apt (Debian / Ubuntu)

### 1. Add the repository key and source

```bash
sudo apt-get install -y gnupg2 curl ca-certificates

curl -fsSL https://tinyhumansai.github.io/openhuman/apt/KEY.gpg \
  | sudo gpg --dearmor -o /etc/apt/keyrings/openhuman.gpg

echo "deb [signed-by=/etc/apt/keyrings/openhuman.gpg arch=amd64] \
  https://tinyhumansai.github.io/openhuman/apt stable main" \
  | sudo tee /etc/apt/sources.list.d/openhuman.list
```

> **arm64:** replace `arch=amd64` with `arch=arm64` or `arch=amd64,arm64`.

### 2. Install

```bash
sudo apt-get update
sudo apt-get install openhuman
```

**Update:**
```bash
sudo apt-get update && sudo apt-get upgrade openhuman
```

**Uninstall:**
```bash
sudo apt-get remove openhuman
# remove repository (optional):
sudo rm /etc/apt/sources.list.d/openhuman.list /etc/apt/keyrings/openhuman.gpg
```

---

## npm

```bash
npm install -g openhuman
```

**Update:**
```bash
npm update -g openhuman
```

**Uninstall:**
```bash
npm uninstall -g openhuman
```

The npm package is a thin wrapper that downloads the platform-native binary on
first install and verifies its SHA-256 checksum before placing it. Node.js ≥ 18
is required; the binary itself has no Node dependency at runtime.

---

## curl / manual install

```bash
curl -fsSL \
  https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh \
  | bash
```

Pass `--dry-run` to preview actions without installing:

```bash
bash scripts/install.sh --dry-run --verbose
```

**Uninstall (manual):**
```bash
rm "$(which openhuman)"
```

---

## Support policy

| Channel | Tier | Maintained by |
|---|---|---|
| Homebrew | **Official** | Core team |
| apt | **Official** | Core team |
| npm | **Official** | Core team |
| curl / install.sh | **Official** | Core team |
| AUR (Arch) | Community | Community PRs |
| Nix | Community | Community PRs |
| Scoop (Windows) | Planned | — |
| Snap / Flatpak | Planned | — |

See [tinyhumansai/openhuman#distribution-backlog](https://github.com/tinyhumansai/openhuman/issues?q=label%3Adistribution-backlog) for the next channels in the pipeline.

---

## Troubleshooting

### macOS Gatekeeper warning

If macOS blocks the binary with *"cannot be opened because the developer cannot be verified"*:

```bash
# Option 1: approve via System Settings → Privacy & Security → Allow Anyway
# Option 2: remove quarantine flag (Homebrew install should handle this automatically)
xattr -d com.apple.quarantine "$(which openhuman)"
```

Binaries installed via Homebrew or the signed `.app` bundle are notarized by
Apple and should not trigger Gatekeeper.

### apt: "NO_PUBKEY" error

Re-import the key:
```bash
curl -fsSL https://tinyhumansai.github.io/openhuman/apt/KEY.gpg \
  | sudo gpg --dearmor -o /etc/apt/keyrings/openhuman.gpg
sudo apt-get update
```

### npm: binary not found after install

The postinstall script may have failed silently. Re-run it manually:
```bash
FORCE_REINSTALL=1 node "$(npm root -g)/openhuman/install.js"
```

Or reinstall cleanly:
```bash
npm uninstall -g openhuman && npm install -g openhuman
```

### Verify checksum manually

Every release asset ships a companion `.sha256` file:

```bash
VERSION=0.49.33
TARGET=x86_64-unknown-linux-gnu
curl -fsSLO "https://github.com/tinyhumansai/openhuman/releases/download/v${VERSION}/openhuman-core-${VERSION}-${TARGET}.tar.gz"
curl -fsSLO "https://github.com/tinyhumansai/openhuman/releases/download/v${VERSION}/openhuman-core-${VERSION}-${TARGET}.tar.gz.sha256"
echo "$(cat openhuman-core-${VERSION}-${TARGET}.tar.gz.sha256)  openhuman-core-${VERSION}-${TARGET}.tar.gz" | sha256sum --check
```

---

## Running from source

The default runtime is **CEF** (bundled Chromium), which requires the **vendored CEF-aware `tauri-cli`** at `app/src-tauri/vendor/tauri-cef/crates/tauri-cli`. The stock `@tauri-apps/cli` does **not** know how to bundle the Chromium Embedded Framework into `OpenHuman.app/Contents/Frameworks/`, so a bundle produced by it panics at startup inside `cef::library_loader::LibraryLoader::new` with `No such file or directory`.

All `cargo tauri` scripts in `app/package.json` (`pnpm dev:app`, `pnpm macos:build:*`, etc.) run [`scripts/ensure-tauri-cli.sh`](../scripts/ensure-tauri-cli.sh) first, which installs the vendored CLI into `~/.cargo/bin/cargo-tauri` on first use. Those scripts also `export CEF_PATH="$HOME/Library/Caches/tauri-cef"` so that **every** `cef-dll-sys` invocation — the main app's and the inner `cargo build` that `tauri-bundler`'s `build.rs` runs to produce the embedded `cef-helper` — resolves to the same CEF binary distribution. Without this, the embedded helper ends up with bindings from a *different* downloaded CEF than the framework loaded at runtime, and helper processes abort with `FATAL: CefApp_0_CToCpp called with invalid version -1`.

If you ever overwrite `cargo-tauri` (e.g. `npm i -g @tauri-apps/cli` or `cargo install tauri-cli`), or switch CEF versions, reinstall with `CEF_PATH` set and force a bundler rebuild (touch forces `tauri-bundler/build.rs` to recompile the embedded cef-helper):

```bash
export CEF_PATH="$HOME/Library/Caches/tauri-cef"
touch app/src-tauri/vendor/tauri-cef/cef-helper/src/*.rs
cargo install --force --locked --path app/src-tauri/vendor/tauri-cef/crates/tauri-cli
```

---

## Release artifacts reference

Each release attaches the following files:

| Artifact | Platform |
|---|---|
| `openhuman-core-<v>-aarch64-apple-darwin.tar.gz` | macOS Apple Silicon |
| `openhuman-core-<v>-x86_64-apple-darwin.tar.gz` | macOS Intel |
| `openhuman-core-<v>-x86_64-unknown-linux-gnu.tar.gz` | Linux x86-64 |
| `openhuman-core-<v>-aarch64-unknown-linux-gnu.tar.gz` | Linux arm64 |
| `OpenHuman_<v>_aarch64.dmg` | macOS desktop app (Apple Silicon) |
| `OpenHuman_<v>_x64.dmg` | macOS desktop app (Intel) |
| `OpenHuman_<v>_amd64.deb` | Linux desktop app (.deb) |
| `OpenHuman_<v>_amd64.AppImage` | Linux desktop app (AppImage) |

Every archive has a corresponding `.sha256` companion file.
