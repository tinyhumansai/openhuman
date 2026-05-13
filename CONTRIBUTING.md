# Contributing to OpenHuman

Thank you for your interest in contributing to OpenHuman. This guide is the fast path for getting a fresh checkout running locally, validating changes, and opening a pull request without having to piece together setup notes from multiple files.

For deeper architecture and subsystem references, use the GitBook under [`gitbooks/developing/`](gitbooks/developing/). For coding-agent and repository-specific implementation rules, see [`AGENTS.md`](AGENTS.md) and [`CLAUDE.md`](CLAUDE.md).

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
- Use [`gitbooks/developing/architecture.md`](gitbooks/developing/architecture.md) for the current system architecture.
- Check [open issues](https://github.com/tinyhumansai/openhuman/issues) and discussions before starting work.
- For security issues, follow [SECURITY.md](SECURITY.md) and do not file public issues.

## Development Setup

### 1. Prerequisites

| Requirement | Version / source of truth | Notes |
| --- | --- | --- |
| Git | Current stable | Required for cloning and updating vendored submodules. |
| Node.js | `>=24.0.0` from [`app/package.json`](app/package.json) | Install the current Node 24 release or newer. |
| pnpm | `pnpm@10.10.0` from [`package.json`](package.json) | The repo enforces pnpm via the root `packageManager` field. |
| Rust | `1.93.0` from [`rust-toolchain.toml`](rust-toolchain.toml) | Install with `rustup`; `rustfmt` and `clippy` are required components. |
| CMake | Current stable | Required by native Rust dependencies such as Whisper bindings. |
| Tauri vendored sources | Git submodules under `app/src-tauri/vendor/` | Required for the CEF-aware Tauri CLI and notification plugin patches. |
| macOS tools | Xcode Command Line Tools | Needed for local desktop builds on macOS. |
| Linux desktop packages | System GTK/WebKit/AppIndicator build deps | Install the package set Tauri requires for your distro before attempting desktop builds. |

#### Platform notes

- **Web-only development** needs Node, pnpm, and the Rust toolchain present in the repo. You can usually ignore desktop-only system packages.
- **Desktop development** needs the vendored Tauri/CEF setup. The preferred entrypoint is `pnpm --filter openhuman-app dev:app`, which ensures the vendored Tauri CLI is installed and configures `CEF_PATH`.
- **Linux desktop builds** require extra system packages beyond Node/Rust. Follow the distro-specific Tauri dependency list before running desktop commands, then use the OpenHuman scripts below. For deeper platform troubleshooting, see [`gitbooks/developing/getting-set-up.md`](gitbooks/developing/getting-set-up.md).
- **Skills development** happens in the separate [`tinyhumansai/openhuman-skills`](https://github.com/tinyhumansai/openhuman-skills) repository. This repo consumes built skill bundles from GitHub or a local override path; it does not vendor the skills source as a submodule.

Example macOS bootstrap with Homebrew:

```bash
brew install node@24 pnpm rustup-init cmake
rustup toolchain install 1.93.0 --profile minimal
rustup component add rustfmt clippy --toolchain 1.93.0
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

Why submodules matter here:

- `app/src-tauri/vendor/tauri-cef`
- `app/src-tauri/vendor/tauri-plugin-notification`

Those vendored trees are part of the current desktop toolchain. If they are missing, desktop builds and Tauri CLI setup will fail.

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

# Preferred desktop development path (sets up vendored Tauri CLI + CEF env)
pnpm --filter openhuman-app dev:app

# Lower-level Tauri command entrypoint
pnpm tauri dev

# Standalone Rust core
cargo run --manifest-path Cargo.toml --bin openhuman-core
```

Which mode to choose:

- `pnpm dev`: frontend-only iteration in the browser.
- `pnpm --filter openhuman-app dev:app`: full desktop app flow with Tauri + CEF.
- `cargo run --bin openhuman-core`: core/RPC work when you want the Rust server without the desktop shell.

### 5. Verify your setup

If setup is correct, these commands should all succeed:

```bash
pnpm typecheck
pnpm lint
pnpm format:check
cargo check --manifest-path Cargo.toml
cargo check --manifest-path app/src-tauri/Cargo.toml
```

If you only changed docs in a normal local workflow, `pnpm format:check` is usually the only validation you need. AI-authored or remote-agent PRs must still follow [`docs/agent-workflows/codex-pr-checklist.md`](docs/agent-workflows/codex-pr-checklist.md) and report any blocked commands with the exact command and error.

### 6. Run tests and checks

| Goal | Command | Notes |
| --- | --- | --- |
| Frontend typecheck | `pnpm typecheck` | Runs the app workspace TypeScript compile check. |
| Frontend lint | `pnpm lint` | ESLint over `app/`. |
| Formatting | `pnpm format:check` | Runs Prettier plus Rust format checks. |
| Frontend unit tests | `pnpm test` or `pnpm test:coverage` | Vitest in `app/`. |
| Rust tests | `pnpm test:rust` | Uses the shared mock backend wrapper. |
| Desktop E2E | `pnpm test:e2e` | Builds the app and runs the desktop flow suites. |
| One-off Vitest debug runs | `pnpm debug unit ...` | Preferred for bounded logs during iteration. |
| One-off Rust debug runs | `pnpm debug rust ...` | Preferred wrapper around focused Rust tests. |

Merge-gate context:

- PRs must meet the checks enforced by CI and keep changed-line coverage at or above 80%.
- For code changes, run the smallest relevant local checks before you push.
- For AI-authored or remote-agent PRs, also follow [`docs/agent-workflows/codex-pr-checklist.md`](docs/agent-workflows/codex-pr-checklist.md).

### 7. Local data and user-facing state

Useful local paths during development:

- `~/.openhuman/`: default workspace for the Rust core and local app data.
- `~/.openhuman-staging/`: staging workspace when `OPENHUMAN_APP_ENV=staging`.
- `app/.env.local`: browser-facing `VITE_*` overrides.
- `.env`: Rust core, Tauri shell, and shared runtime overrides.

Most contributor-visible configuration and state flows are documented in:

- [`gitbooks/developing/getting-set-up.md`](gitbooks/developing/getting-set-up.md)
- [`gitbooks/developing/architecture/frontend.md`](gitbooks/developing/architecture/frontend.md)
- [`gitbooks/developing/architecture/tauri-shell.md`](gitbooks/developing/architecture/tauri-shell.md)

## Project Layout

```text
openhuman/
├── app/                    # React app, Tauri shell, Vitest tests
│   ├── src/
│   ├── src-tauri/
│   └── test/
├── src/                    # Rust core crate and openhuman-core binary
├── docs/                   # Internal and workflow docs
├── gitbooks/developing/    # Contributor-facing architecture and setup guides
├── scripts/                # Dev, test, debug, and automation scripts
├── AGENTS.md               # Coding-agent repo rules
└── CLAUDE.md               # Additional contributor and workflow guidance
```

Short version:

- `app/` is the UI and desktop shell.
- Root `src/` is the Rust core and JSON-RPC surface.
- `gitbooks/developing/` is the canonical place for deeper subsystem docs.

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

Thank you for contributing to OpenHuman.
