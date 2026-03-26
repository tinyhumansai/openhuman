# Contributing to OpenHuman

Thank you for your interest in contributing. This document explains how to get set up, follow our workflow, and submit changes.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Git Workflow](#git-workflow)
- [Making Changes](#making-changes)
- [Submitting Changes](#submitting-changes)
- [Project Conventions](#project-conventions)

## Code of Conduct

This project adheres to the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code.

## Getting Started

- Read the [README](README.md) and [ARCHITECTURE](ARCHITECTURE.md) for context.
- Check [open issues](https://github.com/alphahumanxyz/openhuman/issues) and discussions for ideas and to avoid duplicate work.
- For security issues, see [SECURITY.md](SECURITY.md) — do not report vulnerabilities in public issues.

## Development Setup

### Prerequisites

- [Node.js](https://nodejs.org/) (LTS) and [Yarn](https://yarnpkg.com/)
- [Rust](https://rustup.rs/) (for Tauri and the Rust backend)
- Platform-specific tools for the targets you care about (e.g., Xcode for macOS/iOS, Android SDK for Android)

### Clone and Install

```bash
git clone https://github.com/YOUR_USERNAME/openhuman.git
cd openhuman
yarn install
```

Use your own fork in place of `YOUR_USERNAME` when cloning.

### Run the App

- **Web only**: `yarn dev` (Vite dev server, typically port 1420)
- **Desktop (Tauri)**: `yarn tauri dev` or `yarn dev:app` for enhanced debugging
- **Android**: `yarn tauri android dev`
- **iOS**: `yarn tauri ios dev`

See the main [README](README.md) and project docs for more commands (e.g., `yarn skills:build`, `yarn skills:watch`).

### Environment

Copy or create a `.env` from the documented template and set `VITE_BACKEND_URL`, `VITE_TELEGRAM_*`, and other `VITE_*` variables as needed. Do not commit secrets.

## Git Workflow

- **Fork** the [openhuman](https://github.com/alphahumanxyz/openhuman) repository and work in your fork.
- **Base branch**: All pull requests must target the **`develop`** branch (not `main`).
- **No direct pushes** to the organization repo; all changes come in via pull requests from forks.

### Branch Naming

Use short, descriptive branches, e.g.:

- `fix/telegram-reconnect`
- `feat/settings-dark-mode`
- `docs/contributing-update`

## Making Changes

1. Create a branch from `develop`:
   `git checkout develop && git pull origin develop && git checkout -b fix/your-change`
2. Make your changes. Keep commits focused and messages clear (e.g., “Fix socket reconnect on network drop”).
3. Follow our [project conventions](#project-conventions) and run checks before pushing.

### Running Checks

- **TypeScript**: `yarn compile` (or `tsc --noEmit`)
- **Lint**: `yarn lint` (ESLint); fix auto-fixable issues with `yarn lint:fix`
- **Format**: `yarn format:check`; format with `yarn format` (Prettier)
- **Tests**: `yarn test` (unit), `yarn test:rust` (Rust), `yarn test:e2e` (E2E when applicable)

Pre-commit/pre-push hooks (Husky) run formatting and linting; fix any failures before submitting.

## Submitting Changes

1. Push your branch to your fork:
   `git push origin fix/your-change`
2. Open a **pull request** against **`develop`** in the [openhuman](https://github.com/alphahumanxyz/openhuman) repository.
3. Fill in the PR template (if present): describe what changed, why, and how to test.
4. Link any related issues (e.g., “Fixes #123”).
5. Address review feedback and keep the PR up to date with `develop` (rebase or merge as the project prefers).

Maintainers will review and may request changes. Once approved, your PR will be merged into `develop`.

## Project Conventions

- **State**: Use Redux (and Redux Persist where needed). Avoid `localStorage`/`sessionStorage` for app or feature state; remove existing usage when touching related code.
- **Imports**: Use static `import`/`import type` at the top of the file. No dynamic `import()` for app code; use try/catch around Tauri API calls in non-Tauri environments instead.
- **Code style**: ESLint and Prettier are authoritative. Use type-only imports where appropriate and consolidate imports from the same module.
- **Telegram IDs**: Use the `big-integer` library; do not rely on native JavaScript numbers for Telegram IDs.
- **Tauri**: Commands are in Rust under `src-tauri`; frontend uses `invoke()` from `@tauri-apps/api/core`. Handle missing `window.__TAURI__` where the app can run outside Tauri.
- **Socket events**: Behavior exists in both the TypeScript frontend and the Rust backend. Any new socket event or protocol change must be implemented in both places.
- **Skills**: Follow the V8 runtime and skill manifest rules; respect platform compatibility and the documented bridge/API surface.

For more detail on architecture, patterns, and platform notes, see the project’s internal documentation (e.g., `CLAUDE.md` or equivalent contributor docs).

---

Thank you for contributing to OpenHuman.
