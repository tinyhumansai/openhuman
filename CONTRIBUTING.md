# Contributing to OpenHuman

Thank you for your interest in contributing to OpenHuman. This guide is the fast path for getting a fresh checkout running locally, validating changes, and opening a pull request without having to piece together setup notes from multiple files.

> **New to open source or coding?** Start with [`docs/CONTRIBUTING-BEGINNERS.md`](docs/CONTRIBUTING-BEGINNERS.md) — it walks you through every step from installing tools to opening your first PR.

For deeper architecture and subsystem references, use the GitBook under [`docs/gitbooks/en/developing/`](docs/gitbooks/en/developing/). For coding-agent and repository-specific implementation rules, see [`AGENTS.md`](AGENTS.md) (`CLAUDE.md` is a symlink to the same file).

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Project Layout](#project-layout)
- [Git Workflow](#git-workflow)
- [Making Changes](#making-changes)
- [Submitting Changes](#submitting-changes)
- [Project Conventions](#project-conventions)

## Code of Conduct

This project adheres to the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code.

## Getting Started

- Read the [README](README.md) for product context.
- Use [`docs/gitbooks/en/developing/architecture.md`](docs/gitbooks/en/developing/architecture.md) for the current system architecture.
- Check [open issues](https://github.com/tinyhumansai/openhuman/issues) and [Discussions](https://github.com/tinyhumansai/openhuman/discussions) before starting work.
- Stuck rather than fixing something? [`docs/SUPPORT.md`](docs/SUPPORT.md) routes questions, install trouble, memory behavior, and model problems to the right category; [`docs/community/discussions.md`](docs/community/discussions.md) explains how those threads are triaged.
- For security issues, follow [SECURITY.md](SECURITY.md) and do not file public issues.

## Development Setup

### 1. Prerequisites

| Requirement             | Version / source of truth                                  | Notes                                                                                                                                                        |
| ----------------------- | ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Git                     | Current stable                                             | Required for cloning and updating vendored submodules.                                                                                                       |
| Node.js                 | `>=24.0.0` from [`app/package.json`](app/package.json)     | Install the current Node 24 release or newer.                                                                                                                |
| pnpm                    | `pnpm@10.10.0` from [`package.json`](package.json)         | The repo enforces pnpm via the root `packageManager` field.                                                                                                  |
| Rust                    | `1.99.1` from [`rust-toolchain.toml`](rust-toolchain.toml) | Install with `rustup`; `rustfmt` and `clippy` are required components.                                                                                       |
| CMake                   | Current stable                                             | Optional. Only `aws-lc-sys` (via `rustls`) needs it, and only when the root workspace is built with non-default features; default builds do not use it.      |
| Ninja                   | Current stable                                             | Required on Windows: `scripts/run-dev-win.sh` exits if `ninja` is not on PATH (it pins `CMAKE_GENERATOR=Ninja` for any CMake-driven crate).                  |
| ripgrep (`rg`)          | Current stable                                             | Used by the `lint:commands-tokens` pre-push step (scans `app/src/components/commands/`). Without it, `git push` fails the hook with `rg: command not found`. |
| Vendored module sources | Git submodules under `vendor/`                             | The `tiny*` crates the Rust workspaces depend on by path. Run `git submodule update --init --recursive vendor/` before building.                             |
| macOS tools             | Xcode Command Line Tools                                   | Needed for local desktop builds on macOS.                                                                                                                    |
| Linux desktop packages  | System GTK/WebKit/AppIndicator build deps                  | Install the package set Tauri requires for your distro before attempting desktop builds.                                                                     |

#### Windows-specific setup

Windows requires several additional tools that are not needed on macOS or Linux. Install them in the order listed below, restarting your terminal after each step so PATH changes take effect.

**1. Visual Studio C++ Build Tools**

Rust's `cargo` needs a linker on Windows. The easiest way to get one is during `rustup-init`: select option **1** (Default installation) when prompted, which includes MSVC v143 and the Windows 11 SDK. This is the full Visual Studio installer, not the VS Code lightweight editor — it lives only on `C:` and consumes ~5.4 GB.

**2. Rust**

Install the pinned toolchain and required components with `rustup`:

```powershell
rustup toolchain install 1.99.1 --profile minimal
rustup component add rustfmt clippy --toolchain 1.99.1
```

**3. CMake and Ninja**

`scripts/run-dev-win.sh` pins Ninja as the CMake generator and exits if it cannot find `ninja`. No crate in the default desktop build compiles through CMake today, but the script expects both tools. Install them via winget:

```powershell
winget install Kitware.CMake
winget install --id Ninja-build.Ninja -e
```

**4. Node.js and pnpm**

Install Node.js 24+ and pnpm@10.10.0 as usual.

**Recommended install order**

1. VS Build Tools → restart terminal
2. Rust (`rustup`) → restart terminal
3. CMake + Ninja → restart terminal
4. Node.js + pnpm → restart terminal

**Quick dependency check**

```powershell
# Verify all required tools are reachable
rustc --version
cargo --version
cmake --version
ninja --version
node --version
pnpm --version
```

#### Platform notes

- **Web-only development** needs Node, pnpm, and the Rust toolchain present in the repo. You can usually ignore desktop-only system packages.
- **Desktop development** uses Tauri with Wry as its webview on every platform (`crates/openhuman-app/Cargo.lock` resolves `tauri-runtime-wry`; there is no CEF crate). Use `pnpm --filter openhuman-app dev:app` on macOS and `pnpm dev:app:win` on native Windows.
- **Linux desktop builds** require extra system packages beyond Node/Rust. Follow the distro-specific Tauri dependency list before running desktop commands, then use the OpenHuman scripts below. For deeper platform troubleshooting, see [`docs/gitbooks/en/developing/getting-set-up.md`](docs/gitbooks/en/developing/getting-set-up.md).
- **Windows 10 WSL + classic X11 forwarding** is unsupported for the desktop app. The Tauri desktop flow can hang, render blank windows, or crash before useful app logs are available. Use native Windows development, or Windows 11 WSLg if you need a Linux GUI workflow. OpenHuman logs a startup warning when it detects WSL with `DISPLAY` set but no `WAYLAND_DISPLAY`/WSLg markers.
- **Windows desktop builds** additionally require Visual Studio C++ Build Tools (MSVC v143), CMake, and Ninja. See [Windows-specific setup](#windows-specific-setup) for the full list and install order.
- **macOS desktop builds** require a one-time codesigning cert. After cloning, run `bash scripts/setup-dev-codesign.sh` once to create the local "OpenHuman Dev Signer" self-signed certificate that Tauri uses when bundling dev builds. Without it, `pnpm --filter openhuman-app dev:app` fails at the bundle/sign step with `OpenHuman Dev Signer: no identity found`.
- **Skills development** happens in the separate [`tinyhumansai/skill-registry`](https://github.com/tinyhumansai/skill-registry) repository. This repo consumes built skill bundles from GitHub or a local override path; it does not vendor the skills source as a submodule.

Example macOS bootstrap with Homebrew:

```bash
brew install node@24 pnpm rustup-init ripgrep
rustup toolchain install 1.99.1 --profile minimal
rustup component add rustfmt clippy --toolchain 1.99.1
```

### 2. Clone and install

Fork the upstream repository on GitHub first if you plan to submit changes, then clone your fork:

```bash
git clone git@github.com:YOUR_USERNAME/openhuman.git
cd openhuman
git remote add upstream git@github.com:tinyhumansai/openhuman.git
git submodule update --init --recursive
pnpm install
```

Why submodules matter here: `vendor/` holds the `tiny*` crates (`tinyagents`, `tinymemory`, `tinybus`, `tinyhumans-sdk`, and so on). `crates/openhuman-core/Cargo.toml` and `crates/openhuman-app/Cargo.toml` depend on them with `path = "../../vendor/..."`, and the root and app `[patch]` tables redirect the git/crates.io versions of `tinymemory-api`, `tinyinference`, `tinyflows`, and `tinychannels` into the same tree. If the submodules are missing, `cargo check` and `cargo build` fail to resolve those paths.

### 3. Configure for development

OpenHuman uses two environment templates:

- Root [`.env.example`](.env.example): Rust core, Tauri shell, shared runtime settings.
- [`app/.env.example`](app/.env.example): frontend `VITE_*` variables for the web app.

Copy them to local-only files before editing:

```bash
cp .env.example .env
cp app/.env.example app/.env.local
```

Minimal configuration guidance:

- **Web UI / frontend work**: the defaults in `app/.env.local` are usually enough for local startup. Set `VITE_BACKEND_URL` only if you need a non-production backend in web mode.
- **Desktop work**: leave `OPENHUMAN_CORE_TOKEN` blank for local child-mode development unless you are intentionally wiring an external core. The shell manages the embedded core token flow.
- **Core RPC / standalone core work**: `OPENHUMAN_CORE_PORT=7788` and `OPENHUMAN_CORE_RPC_URL=http://127.0.0.1:7788/rpc` are already documented in the root template and are the normal local defaults.
- **Skills development**: use `SKILLS_REGISTRY_URL` or `SKILLS_LOCAL_DIR` from the root template when pointing the app at a local built skills checkout.

Never commit `.env`, `app/.env.local`, tokens, or other secrets.

### 4. Bootstrap commands

These commands cover the most common local workflows from the repository root:

```bash
# Install workspace dependencies
pnpm install

# Web-only development (Vite dev server)
pnpm dev

# Preferred macOS desktop development path (Tauri + Wry)
pnpm --filter openhuman-app dev:app

# Preferred native Windows desktop development path (run from PowerShell)
pnpm dev:app:win

# Lower-level Tauri command entrypoint
pnpm tauri dev

# Standalone Rust core
cargo run --manifest-path Cargo.toml --bin openhuman-core
```

Which mode to choose:

- `pnpm dev`: frontend-only iteration in the browser.
- `pnpm --filter openhuman-app dev:app`: full desktop app flow with Tauri + Wry on macOS.
- `pnpm dev:app:win`: full desktop app flow on native Windows. This invokes the repository's Git Bash bootstrap (`scripts/run-dev-win.sh`) to configure the MSVC toolchain, Ninja, and PATH before launching the local `@tauri-apps/cli` (`tauri dev`). The script still prints a `CEF_RUNTIME_PATH` warning left over from the pre-Wry runtime; it is harmless.
- `cargo run --bin openhuman-core`: core/RPC work when you want the Rust server without the desktop shell.

#### Windows desktop troubleshooting

The default development port is `1420`. Hyper-V or WSL can reserve ranges that include this port. Check the excluded TCP ranges from an elevated PowerShell prompt:

```powershell
netsh interface ipv4 show excludedportrange protocol=tcp
```

If `1420` is excluded or already in use, choose a port `N` such that both `N` and `N + 1` are available and outside the listed ranges; use a value below `65535`. `dev:app:win` applies `OPENHUMAN_DEV_PORT` to both Vite and Tauri, so their URLs remain synchronized:

```powershell
$env:OPENHUMAN_DEV_PORT = "14320"
pnpm dev:app:win
```

On systems using a non-UTF-8 code page, MSVC-compiled native dependencies can fail with errors `C4819` and `C2220`. Opt into UTF-8 for the current PowerShell session before starting the app:

```powershell
$env:CL = "/utf-8"
pnpm dev:app:win
```

### 5. Verify your setup

If setup is correct, these commands should all succeed:

```bash
pnpm typecheck
pnpm lint
pnpm format:check
cargo check --manifest-path Cargo.toml
cargo check --manifest-path crates/openhuman-app/Cargo.toml
```

If you only changed docs in a normal local workflow, `pnpm format:check` is usually the only validation you need. AI-authored or remote-agent PRs must still fill in the AI Authored PR Metadata section of the PR template and report any blocked commands with the exact command and error.

### 6. Run tests and checks

| Goal                      | Command                             | Notes                                            |
| ------------------------- | ----------------------------------- | ------------------------------------------------ |
| Frontend typecheck        | `pnpm typecheck`                    | Runs the app workspace TypeScript compile check. |
| Frontend lint             | `pnpm lint`                         | ESLint over `app/`.                              |
| Formatting                | `pnpm format:check`                 | Runs Prettier plus Rust format checks.           |
| Frontend unit tests       | `pnpm test` or `pnpm test:coverage` | Vitest in `app/`.                                |
| Rust tests                | `pnpm test:rust`                    | Uses the shared mock backend wrapper.            |
| Desktop E2E               | `pnpm test:e2e`                     | Builds the app and runs the desktop flow suites. |
| One-off Vitest debug runs | `pnpm debug unit ...`               | Preferred for bounded logs during iteration.     |
| One-off Rust debug runs   | `pnpm debug rust ...`               | Preferred wrapper around focused Rust tests.     |

Merge-gate context:

- PRs must meet the checks enforced by CI and keep changed-line coverage at or above 80%.
- For code changes, run the smallest relevant local checks before you push.
- For AI-authored or remote-agent PRs, also fill in the AI Authored PR Metadata section of the PR template.

### 7. Local data and user-facing state

Useful local paths during development:

- `~/.openhuman/`: default workspace for the Rust core and local app data.
- `~/.openhuman-staging/`: staging workspace when `OPENHUMAN_APP_ENV=staging`.
- `app/.env.local`: browser-facing `VITE_*` overrides.
- `.env`: Rust core, Tauri shell, and shared runtime overrides.

Most contributor-visible configuration and state flows are documented in:

- [`docs/gitbooks/en/developing/getting-set-up.md`](docs/gitbooks/en/developing/getting-set-up.md)
- [`docs/gitbooks/en/developing/architecture/frontend.md`](docs/gitbooks/en/developing/architecture/frontend.md)
- [`docs/gitbooks/en/developing/architecture/tauri-shell.md`](docs/gitbooks/en/developing/architecture/tauri-shell.md)

## Project Layout

```text
openhuman/
├── app/                    # React frontend (app/src), Vitest + E2E tests (app/test)
├── crates/
│   ├── openhuman-app/      # Thin Tauri desktop host (separate workspace)
│   ├── openhuman-core/     # Rust core: domains under src/<domain>/, RPC server, `openhuman-core` CLI
│   ├── openhuman-embed/    # Library facade for embedding the core
│   ├── openhuman-rpc/      # Shared RPC contracts + HTTP client (app, TUI)
│   └── openhuman-tui/      # Terminal frontend
├── tests/                  # Rust integration and JSON-RPC E2E tests (explicit [[test]] targets)
├── examples/               # Embedding examples (explicit [[example]] targets)
├── vendor/                 # Recursive git submodules (tinyagents, tinymemory, ...)
├── docs/                   # Internal maintainer docs
├── docs/gitbooks/en/               # Public product and contributor docs
├── scripts/                # Dev, test, CI, and release scripts
└── AGENTS.md               # Repo rules for contributors and coding agents (CLAUDE.md symlinks here)
```

There is no `app/src-tauri/`; the app workspace uses `app/src-tauri-mobile/` and
`app/src-tauri-web/` for those targets, and the desktop host lives in
`crates/openhuman-app/`. Root `Cargo.toml` is a virtual workspace with members
`openhuman-core`, `openhuman-embed`, `openhuman-rpc`, and `openhuman-tui`;
`crates/openhuman-app` is deliberately excluded and builds as its own
workspace (`cargo check --manifest-path crates/openhuman-app/Cargo.toml`).

Short version:

- `app/` is the UI and desktop shell.
- `crates/openhuman-core/` is the Rust core and JSON-RPC surface; see the
  "Rust domain structure" section of AGENTS.md.
- `docs/gitbooks/en/developing/` is the canonical place for deeper subsystem docs.

## Git Workflow

- Fork [tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman) and push branches to your fork.
- Pull requests target the upstream `main` branch.
- Do not push directly to upstream unless you are explicitly authorized to do so.

### Branch naming

Use a short descriptive branch name, for example:

- `fix/socket-reconnect`
- `feat/settings-shortcuts`
- `docs/contributing-setup`

### Starting a branch

```bash
git fetch upstream
git checkout main
git pull --ff-only upstream main
git checkout -b docs/your-change
```

## Making Changes

1. Start from `main` and create a focused branch.
2. Keep the diff small and scoped to the issue you are solving.
3. Run the smallest relevant checks locally before pushing.
4. Update docs with code whenever behavior, commands, or contributor workflow changes.

### Workflow sanity checklist

- Verify the command you are documenting exists in the current repo.
- Prefer source-of-truth files such as `package.json`, `app/package.json`, `Cargo.toml`, `rust-toolchain.toml`, and the env templates over older prose docs.
- Link to GitBook chapters for deeper architecture instead of duplicating large internal explanations.

## Submitting Changes

1. Push your branch to your fork.
2. Open a pull request against `tinyhumansai/openhuman:main`.
3. Fill in [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md) completely.
4. Link the issue using a closing keyword such as `Closes #1441`.
5. Call out any blocked validation commands with the exact command and error.

If you are contributing through a coding agent or remote environment, include the metadata required by the PR template and the Codex PR checklist.

## Project Conventions

- Use Redux and existing app state patterns instead of adding new ad hoc browser storage.
- Treat Rust core logic as the source of truth; avoid re-implementing business rules in the Tauri shell.
- Use the controller registry and domain module structure described in [`AGENTS.md`](AGENTS.md) for new Rust functionality.
- Keep logs grep-friendly and avoid logging secrets, tokens, or full PII.
- Follow ESLint, Prettier, and Rust formatting output as authoritative.

### UI translations

All locales use the same workflow. Use `useT()` for UI text and treat
`app/src/lib/i18n/en.ts` as the key source. Preserve existing good translations,
translate new or changed English meanings, and remove keys retired from English.
`scripts/apply-i18n-translations.ts` applies translation JSON files, retains
existing values, removes obsolete keys, and writes keys in English order. Supply
real translations for new strings; its English fallback is not a finished translation.
Run `pnpm i18n:check`, `pnpm i18n:english:check`, the i18n coverage test, and Prettier.
Preserve interpolation placeholders exactly, including repeated placeholders and
double braces. Keep product names, paths, commands, and technical identifiers intact.

#### Japanese UI copy

Review text where it appears: a button, setting label, status, empty state, or
confirmation dialog. Prefer short action labels and plain, polite sentences.
Remove roundabout wording, not information: keep conditions, units, upload
destinations, consent requirements, and deletion scope. Errors should describe
what happened without guessing a cause. For example, keep "no answer" distinct
from "no data" or a connection failure.

| Context | Prefer | Avoid |
| --- | --- | --- |
| Save button | 保存 | 変更内容の保存を実行 |
| Saved status | 保存しました | 正常に保存されました |
| Available action | 変更できます | 変更することができます |
| Delete a stored memory item | メモリーから削除 | 忘れる |

Use these terms consistently on the Memory screens. Translate by meaning rather
than substituting words mechanically; source connections and answer citations are
different concepts.

| Concept | Japanese |
| --- | --- |
| Memory / memory engine | メモリー / メモリーエンジン |
| Learning / preference | 学習内容 / 好み |
| Brain / belief / memory pack | ブレイン / 信念 / メモリーパック |
| Source to sync / answer citation | ソース / 出典 |
| Context brief | コンテキスト要約; 要約 when the context is clear |
| Sync / import | 同期 / インポート |
| Endpoint URL / API key | 接続先URL / APIキー |
| Conversation / thread / turn | 会話 / スレッド / ターン |

Keep backend count units such as ターン; do not reinterpret them as messages or
conversations. This is contributor writing guidance, not a separate Japanese key
matcher, fallback, or validation rule.

Thank you for contributing to OpenHuman.
