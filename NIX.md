# Building OpenHuman with Nix

OpenHuman ships a [`flake.nix`](./flake.nix) that provides a reproducible dev
shell and builds for the two terminal-facing hosts, `openhuman-core` (the
JSON-RPC server / agent harness) and `openhuman-tui` (the terminal client).

Everything below assumes flakes are enabled:

```bash
experimental-features = nix-command flakes
```

## Quick start

```bash
# Enter a shell with the pinned Rust toolchain, Node, pnpm and the system
# libraries the core links against.
nix develop "git+file://$PWD?submodules=1"

# Build the core binary.
nix build "git+file://$PWD?submodules=1#openhuman-core"
./result/bin/openhuman-core --help

# Build the TUI.
nix build "git+file://$PWD?submodules=1#openhuman-tui"

# Or run the core straight from a checkout / remote ref.
nix run "github:tinyhumansai/openhuman?submodules=1#openhuman-core" -- serve
```

## Why `?submodules=1` matters

The Cargo workspace links ~19 `tiny*` crates out of `vendor/`, and several of
those vendor a **nested** copy of `tinytools`, `tinyinference` and
`tinystoragedrivers`. The root `Cargo.toml` redirects git/registry
dependencies to those paths with `[patch]` tables, so the submodule contents are
not optional — without them cargo cannot even resolve the graph.

Nix's flake-source copier deliberately **drops git submodule contents** unless
asked for them. A bare `nix build .#openhuman-core` therefore fails with:

```
error: Path 'vendor' in the repository "..." is not tracked by Git.
```

Pass one of:

| Ref | What it copies | When to use |
| --- | --- | --- |
| `git+file://$PWD?submodules=1` | tracked files **+ submodule contents** | local checkout, the default |
| `path:.` | the working tree verbatim (submodules included) | submodules already checked out and you want uncommitted edits visible |
| `github:tinyhumansai/openhuman?submodules=1` | remote checkout + submodules | consuming without cloning |

If you cloned without submodules, populate them first:

```bash
bash scripts/ci/checkout-submodules.sh
```

## What the flake provides

### `devShells.default`

A shell that can run the repo's own commands (`cargo check`, `cargo nextest`,
`pnpm install`, `pnpm --filter openhuman-app build`) without any host
setup. It pins:

* **Rust** from [`rust-toolchain.toml`](./rust-toolchain.toml) via
  `rust-overlay` — the repo requires **1.96.1** (`rusqlite` 0.40 /
  `libsqlite3-sys` 0.38 need the `cfg_select!` macro stabilized in 1.96), and
  rust-overlay honours that instead of using nixpkgs' default toolchain.
* **Node 24** and **pnpm** (matching `package.json`'s `engines` /
  `packageManager`).
* The **native build inputs**: `pkg-config`, `cmake`, `clang`, `mold`, `perl`.
* The **system libraries** the core links unconditionally: OpenSSL, ALSA
  (`cpal`), `libxdo` (`enigo`), `libevdev` + `libXtst` (`rdev`), the X11 set,
  `libxkbcommon`, `libsecret` + `dbus` (`keyring`).

### `packages.openhuman-core` / `packages.openhuman-tui`

Built with [crane](https://github.com/ipetkov/crane). `buildDepsOnly` compiles
the dependency graph once into a cached `cargoArtifacts` derivation, so editing
OpenHuman source only rebuilds the workspace crates. Both use `--locked`, so
they build exactly the `Cargo.lock` in the tree.

### `formatter`

`nix fmt` runs `nixfmt` over the Nix files.

## CI

[`.github/workflows/nix-flake.yml`](./.github/workflows/nix-flake.yml) keeps the
flake honest without a multi-hour build: it runs `nix flake check --no-build
--all-systems`, builds the dev shell's inputs, and asserts `flake.lock` is
current. A full `nix build` of the core is left to users and self-hosted
builders.

## Notes & caveats

* **The desktop app is not packaged here.** `crates/openhuman-app` is a
  separate Cargo world (its own lockfile, GTK/WebKit/Tauri) and is excluded
  from the root workspace on purpose. Building it needs a WebKitGTK/Tauri
  toolchain and is out of scope for this flake; use `pnpm --filter
  openhuman-app tauri build` from the dev shell if you need it.
* **First build is expensive.** The core pulls ~800 crates and compiles the
  bundled SQLite. Expect a long first `nix build`; subsequent builds reuse the
  dependency derivation.
* **Sandboxed build.** The build runs without network access; all crates come
  from `Cargo.lock` and the vendored submodules. `openssl` is taken from
  nixpkgs (`OPENSSL_NO_VENDOR=1`) rather than compiled from source.
* **Linux is the supported system today.** The flake evaluates on `aarch64`
  and `darwin` too, but the system-library lists (ALSA, X11, `libxdo`) are the
  Linux set; macOS/Windows would need their own `buildInputs`.
