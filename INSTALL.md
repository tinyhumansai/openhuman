# Installing OpenHuman

## Download the app (recommended)

Download the desktop app from [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman?utm_source=github&utm_medium=readme) or the [latest release](https://github.com/tinyhumansai/openhuman/releases/latest).

| Platform | File |
| --- | --- |
| macOS (Apple Silicon) | `OpenHuman_<version>_aarch64.dmg` |
| macOS (Intel) | `OpenHuman_<version>_x64.dmg` |
| Windows | `OpenHuman_<version>_x64_en-US.msi` (or the `x64-setup.exe`) |
| Debian / Ubuntu | `OpenHuman_<version>_amd64.deb` or `_arm64.deb` |
| Other Linux | `OpenHuman_<version>_amd64.AppImage` or `_aarch64.AppImage` |

On Debian and Ubuntu, install the `.deb` with `apt-get` so it pulls in the system libraries it needs:

```bash
sudo apt-get install -y --no-install-recommends ./OpenHuman_*_amd64.deb
```

Use `arm64` in place of `amd64` on ARM machines.

## Install script

The install script works out your platform, downloads the matching file from the latest release, checks its SHA-256 against the one GitHub publishes for the release, and installs it.

```bash
# macOS and Linux
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

What it installs:

- macOS: the `.dmg` (or the `.app.tar.gz` on Intel).
- Windows: the `.msi`, falling back to the `.exe` installer.
- Debian and Ubuntu: the `.deb`, through `apt-get`. Set `OPENHUMAN_INSTALLER_LINUX_PACKAGE=appimage` to get the AppImage instead.
- Other Linux: the AppImage.

To see what it would do without changing anything, run it with `--dry-run`:

```bash
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash -s -- --dry-run
```

The script checks the file it downloads, but the script itself is not signed. It is served live from GitHub, so if you want to be sure of what you run, download it, read it, then run it.

## Homebrew

A Homebrew cask also exists (`brew install --cask openhuman`). It can lag a release behind, so prefer the download or the script above.

## Nix (flake)

A `flake.nix` is included for Nix users. It provides a dev shell with the pinned
Rust/Node toolchain and the system libraries the core links, plus builds for the
two terminal hosts (`openhuman-core`, `openhuman-tui`). The desktop app is not
packaged by the flake.

```bash
# Build and run the core straight from a checkout
nix build "git+file://$PWD?submodules=1#openhuman-core"
./result/bin/openhuman-core --help

# Dev shell
nix develop "git+file://$PWD?submodules=1"
```

The `?submodules=1` is required: OpenHuman vendors its `tiny*` crates under
`vendor/` as git submodules and Nix's flake copier drops submodule contents by
default. See [NIX.md](./NIX.md) for details.

## Troubleshooting

The AppImage can crash on launch under Wayland, miss host libraries such as `libgbm.so.1`, or fail on Arch-based distros with `sharun: Interpreter not found!`. See [#2463](https://github.com/tinyhumansai/openhuman/issues/2463) for the cause and workarounds. On Debian and Ubuntu, the `.deb` avoids these problems because apt resolves the dependencies.
